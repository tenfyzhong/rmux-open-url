use std::collections::BTreeSet;
use std::io::{self, Stdout, Write};
use std::process::Command;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use clap::Parser;
use crossterm::cursor::{Hide, MoveTo, Show};
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::queue;
use crossterm::style::{Attribute, Color, Print, SetAttribute, SetForegroundColor};
use crossterm::terminal::{
    Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode,
    enable_raw_mode, size,
};
use rmux_open_url::{
    Extractor, build_copy_action, build_open_action, filter_candidates, parse_show_options,
    resolve_copy_command, resolve_opener, run_action, spawn_detached, strip_ansi,
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

#[derive(Debug, Parser)]
#[command(
    version,
    about = "Query and open URLs visible on the rmux screen",
    after_help = "Enter opens the selected URLs in the browser; Tab toggles multi-select;\n\
                  Ctrl-y copies instead of opening; Esc cancels."
)]
struct Args {
    /// Target rmux pane. The key binding passes #{pane_id} here.
    #[arg(long, env = "RMUX_OPEN_URL_PANE")]
    pane: String,

    /// rmux executable used for pane capture and the copy fallback.
    #[arg(long, default_value = "rmux")]
    rmux: String,

    /// Capture this many lines of history instead of just the visible screen.
    #[arg(long)]
    limit: Option<u32>,

    /// Command used to open URLs ({} is replaced with the URL).
    /// Falls back to the @open-url-open option.
    #[arg(long)]
    open: Option<String>,

    /// Command used to copy URLs to the clipboard.
    /// Falls back to the @open-url-copy-cmd option.
    #[arg(long)]
    copy_cmd: Option<String>,

    /// Custom regex pattern for URL extraction.
    /// Falls back to the @open-url-custom-pat option.
    #[arg(long)]
    custom_pat: Option<String>,

    /// Replacement for --custom-pat ($0 is the whole match).
    /// Falls back to the @open-url-custom-sub option.
    #[arg(long)]
    custom_sub: Option<String>,
}

fn main() {
    if let Err(error) = run(Args::parse()) {
        eprintln!("rmux-open-url: {error:#}");
        std::process::exit(1);
    }
}

fn run(args: Args) -> Result<()> {
    let options = load_options(&args.rmux);

    let limit = args
        .limit
        .or_else(|| option_value(&options, "@open-url-history-limit").and_then(|v| v.parse().ok()));
    let text = strip_ansi(&capture_pane(&args.rmux, &args.pane, limit)?);

    let mut extractor = Extractor::with_defaults()?;
    if let Some(pattern) = args
        .custom_pat
        .clone()
        .or_else(|| option_value(&options, "@open-url-custom-pat"))
        .filter(|value| !value.is_empty())
    {
        let substitution = args
            .custom_sub
            .clone()
            .or_else(|| option_value(&options, "@open-url-custom-sub"));
        extractor.with_custom(&pattern, substitution.as_deref())?;
    }
    let urls = extractor.extract(&text);

    if urls.is_empty() {
        let _ = Command::new(&args.rmux)
            .args(["display-message", "rmux-open-url: no URLs found"])
            .status();
        return Ok(());
    }

    let copy_command = args
        .copy_cmd
        .clone()
        .or_else(|| option_value(&options, "@open-url-copy-cmd"))
        .unwrap_or_else(|| resolve_copy_command(None, &args.rmux));

    let Some(selected) = run_ui(&urls, &copy_command)? else {
        return Ok(());
    };

    let open_command = match args
        .open
        .clone()
        .or_else(|| option_value(&options, "@open-url-open"))
    {
        Some(command) => command,
        None => resolve_opener(None)?,
    };
    for index in selected {
        let input = build_open_action(&open_command, &urls[index])?;
        spawn_detached(&input)?;
    }
    Ok(())
}

/// Load `@open-url-*` options from the rmux server, mirroring how
/// the reference tmux plugin reads its options from tmux. Failures are
/// non-fatal: the command line flags and built-in defaults still apply.
fn load_options(rmux: &str) -> Vec<(String, String)> {
    let output = match Command::new(rmux).args(["show-options", "-g"]).output() {
        Ok(output) => output,
        Err(error) => {
            eprintln!("rmux-open-url: load options: {error}");
            return Vec::new();
        }
    };
    if !output.status.success() {
        eprintln!(
            "rmux-open-url: load options: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
        return Vec::new();
    }
    let Ok(output) = String::from_utf8(output.stdout) else {
        eprintln!("rmux-open-url: load options: output is not UTF-8");
        return Vec::new();
    };
    parse_show_options(&output)
}

fn option_value(options: &[(String, String)], name: &str) -> Option<String> {
    options
        .iter()
        .find(|(existing, _)| existing == name)
        .map(|(_, value)| value.clone())
        .filter(|value| !value.is_empty())
}

fn capture_pane(rmux: &str, pane: &str, limit: Option<u32>) -> Result<String> {
    let mut command = Command::new(rmux);
    command.args(["capture-pane", "-J", "-p", "-e", "-t", pane]);
    if let Some(limit) = limit {
        command.args(["-S", &format!("-{limit}")]);
    }
    let output = command
        .output()
        .with_context(|| format!("run {rmux:?} capture-pane"))?;
    if !output.status.success() {
        bail!(
            "capture pane {pane:?}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    String::from_utf8(output.stdout).context("captured pane is not UTF-8")
}

// ---------------------------------------------------------------------------
// Interactive URL picker
// ---------------------------------------------------------------------------

struct Terminal {
    stdout: Stdout,
}

impl Terminal {
    fn enter() -> Result<Self> {
        enable_raw_mode().context("enable raw terminal mode")?;
        let mut stdout = io::stdout();
        if let Err(error) = execute!(stdout, EnterAlternateScreen, Hide) {
            let _ = disable_raw_mode();
            return Err(error).context("enter alternate screen");
        }
        Ok(Self { stdout })
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        let _ = execute!(self.stdout, Show, LeaveAlternateScreen);
        let _ = disable_raw_mode();
    }
}

struct UiState {
    urls: Vec<String>,
    filtered: Vec<usize>,
    query: String,
    selected: BTreeSet<usize>,
    cursor: usize,
    notice: Option<String>,
}

impl UiState {
    fn new(urls: Vec<String>) -> Self {
        let filtered = (0..urls.len()).collect();
        Self {
            urls,
            filtered,
            query: String::new(),
            selected: BTreeSet::new(),
            cursor: 0,
            notice: None,
        }
    }

    fn refilter(&mut self) {
        self.filtered = filter_candidates(&self.query, &self.urls);
        self.cursor = 0;
    }

    fn type_char(&mut self, ch: char) {
        self.query.push(ch);
        self.refilter();
    }

    fn backspace(&mut self) {
        self.query.pop();
        self.refilter();
    }

    fn clear_query(&mut self) {
        self.query.clear();
        self.refilter();
    }

    fn cursor_up(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    fn cursor_down(&mut self) {
        if self.cursor + 1 < self.filtered.len() {
            self.cursor += 1;
        }
    }

    fn page_up(&mut self, viewport: usize) {
        self.cursor = self
            .cursor
            .saturating_sub(viewport.saturating_sub(1).max(1));
    }

    fn page_down(&mut self, viewport: usize) {
        self.cursor = (self.cursor + viewport.saturating_sub(1).max(1))
            .min(self.filtered.len().saturating_sub(1));
    }

    fn home(&mut self) {
        self.cursor = 0;
    }

    fn end(&mut self) {
        self.cursor = self.filtered.len().saturating_sub(1);
    }

    fn toggle(&mut self) {
        if self.filtered.is_empty() {
            return;
        }
        let index = self.filtered[self.cursor.min(self.filtered.len() - 1)];
        if !self.selected.insert(index) {
            self.selected.remove(&index);
        }
    }

    /// Enter confirms the selected URLs, or all filtered matches when nothing
    /// is selected.
    fn confirm_open(&self) -> Vec<usize> {
        if self.selected.is_empty() {
            self.filtered.clone()
        } else {
            self.selected.iter().copied().collect()
        }
    }

    /// Ctrl-y copies the selected URLs, or the current line when nothing is
    /// selected.
    fn confirm_copy(&self) -> Vec<usize> {
        if self.selected.is_empty() {
            if self.filtered.is_empty() {
                Vec::new()
            } else {
                vec![self.filtered[self.cursor.min(self.filtered.len() - 1)]]
            }
        } else {
            self.selected.iter().copied().collect()
        }
    }
}

fn run_ui(urls: &[String], copy_command: &str) -> Result<Option<Vec<usize>>> {
    let mut terminal = Terminal::enter()?;
    let mut state = UiState::new(urls.to_vec());
    render(&mut terminal.stdout, &state)?;

    loop {
        if !event::poll(Duration::from_secs(30)).context("poll terminal input")? {
            continue;
        }
        let event = event::read().context("read terminal input")?;
        match event {
            Event::Resize(_, _) => render(&mut terminal.stdout, &state)?,
            Event::Key(key) if key.kind == KeyEventKind::Press => {
                let control = key.modifiers.contains(KeyModifiers::CONTROL);
                let mut copied = false;
                match key.code {
                    KeyCode::Esc => return Ok(None),
                    KeyCode::Char('c') if control => return Ok(None),
                    KeyCode::Enter => return Ok(Some(state.confirm_open())),
                    KeyCode::Tab | KeyCode::BackTab => state.toggle(),
                    KeyCode::Char('y') if control => {
                        let indices = state.confirm_copy();
                        if !indices.is_empty() {
                            let selection = indices
                                .iter()
                                .map(|&index| urls[index].as_str())
                                .collect::<Vec<_>>()
                                .join("\n");
                            match build_copy_action(copy_command, &selection)
                                .and_then(|input| run_action(&input))
                            {
                                Ok(()) => {
                                    state.notice = Some(format!("Copied {} URL(s)", indices.len()));
                                }
                                Err(error) => {
                                    state.notice = Some(format!("copy failed: {error}"));
                                }
                            }
                            copied = true;
                        }
                    }
                    KeyCode::Char('u') if control => state.clear_query(),
                    KeyCode::Char('a') if control => state.home(),
                    KeyCode::Char('e') if control => state.end(),
                    KeyCode::Char('j') | KeyCode::Char('n') if control => state.cursor_down(),
                    KeyCode::Char('k') | KeyCode::Char('p') if control => state.cursor_up(),
                    KeyCode::Char('\n') => state.cursor_down(),
                    KeyCode::Char(ch) if !control => state.type_char(ch),
                    KeyCode::Backspace => state.backspace(),
                    KeyCode::Up => state.cursor_up(),
                    KeyCode::Down => state.cursor_down(),
                    KeyCode::PageUp => state.page_up(viewport_rows()),
                    KeyCode::PageDown => state.page_down(viewport_rows()),
                    KeyCode::Home => state.home(),
                    KeyCode::End => state.end(),
                    _ => {}
                }
                if !copied {
                    state.notice = None;
                }
                render(&mut terminal.stdout, &state)?;
            }
            _ => {}
        }
    }
}

fn viewport_rows() -> usize {
    size()
        .map(|(_, height)| height as usize)
        .unwrap_or(10)
        .saturating_sub(2)
}

#[derive(Clone, Copy)]
enum Style {
    Help,
    Normal,
    Current,
    Selected,
    SelectedCurrent,
}

fn style_colors(style: Style) -> (Color, bool) {
    match style {
        Style::Help => (Color::DarkGrey, false),
        Style::Normal => (Color::White, false),
        Style::Current => (Color::Green, true),
        Style::Selected => (Color::Yellow, false),
        Style::SelectedCurrent => (Color::Yellow, true),
    }
}

fn truncate_to_width(text: &str, max: usize) -> String {
    if text.width() <= max {
        return text.to_owned();
    }
    let mut out = String::new();
    let mut width = 0usize;
    for ch in text.chars() {
        let ch_width = ch.width().unwrap_or(0);
        if width + ch_width > max {
            break;
        }
        out.push(ch);
        width += ch_width;
    }
    out
}

fn paint_line(stdout: &mut Stdout, y: u16, text: &str, width: u16, style: Style) -> Result<()> {
    let (color, bold) = style_colors(style);
    queue!(
        stdout,
        MoveTo(0, y),
        SetAttribute(Attribute::Reset),
        SetForegroundColor(color),
        SetAttribute(if bold {
            Attribute::Bold
        } else {
            Attribute::NormalIntensity
        }),
        Print(truncate_to_width(text, width as usize)),
    )?;
    Ok(())
}

fn render(stdout: &mut Stdout, state: &UiState) -> Result<()> {
    let (width, height) = size().context("read terminal size")?;
    let width = width as usize;
    let height = height as usize;
    queue!(stdout, Clear(ClearType::All))?;

    let help = state
        .notice
        .clone()
        .unwrap_or_else(|| "enter:open  tab:select  ctrl-y:copy  esc:quit".to_owned());
    paint_line(stdout, 0, &help, width as u16, Style::Help)?;

    // The filtered list sits between the help line and the prompt, with the
    // current match directly above the prompt.
    let viewport = height.saturating_sub(2);
    if viewport > 0 && !state.filtered.is_empty() {
        let cursor = state.cursor.min(state.filtered.len() - 1);
        // Show up to `viewport` rows; when the list overflows, scroll so the
        // current row sits directly above the prompt.
        let first = if state.filtered.len() <= viewport {
            0
        } else {
            cursor.saturating_sub(viewport - 1)
        };
        let mut row = 1u16;
        for &index in state.filtered.iter().skip(first).take(viewport) {
            let is_current = index == state.filtered[cursor];
            let is_selected = state.selected.contains(&index);
            let style = match (is_selected, is_current) {
                (true, true) => Style::SelectedCurrent,
                (true, false) => Style::Selected,
                (false, true) => Style::Current,
                (false, false) => Style::Normal,
            };
            let marker = if is_current { ">" } else { " " };
            // Numbers are stable across filtering.
            let line = format!("{marker}{:>3}  {}", index + 1, state.urls[index]);
            paint_line(stdout, row, &line, width as u16, style)?;
            row += 1;
        }
    } else if viewport > 0 && width > 4 {
        paint_line(
            stdout,
            (height / 2) as u16,
            "no match",
            width as u16,
            Style::Help,
        )?;
    }

    let count = format!("{}/{}", state.filtered.len(), state.urls.len());
    let prompt_y = height.saturating_sub(1) as u16;
    let query_max = width.saturating_sub(count.len() + 3).max(1);
    let query = truncate_to_width(&state.query, query_max);
    queue!(
        stdout,
        MoveTo(0, prompt_y),
        SetAttribute(Attribute::Reset),
        SetForegroundColor(Color::Green),
        SetAttribute(Attribute::Bold),
        Print("> "),
        SetAttribute(Attribute::NormalIntensity),
        SetForegroundColor(Color::White),
        Print(&query),
        MoveTo(width.saturating_sub(count.len()) as u16, prompt_y),
        SetForegroundColor(Color::DarkGrey),
        Print(&count),
        MoveTo((2 + query.width()) as u16, prompt_y),
        Show,
    )?;
    stdout.flush().context("draw open-url overlay")
}
