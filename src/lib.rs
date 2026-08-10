//! Core logic for rmux-open-url.
//!
//! A Rust port of the URL extraction and interactive selection workflow of
//! the reference tmux plugin, adapted for [rmux]. This crate provides the
//! extraction patterns, ANSI stripping, fuzzy filtering, option parsing, and
//! action handling shared by the CLI and its tests.
//!
//! [rmux]: https://rmux.io

use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::OnceLock;

use anyhow::{Context, Result, anyhow, bail};
use regex::Regex;

// ---------------------------------------------------------------------------
// URL extraction
// ---------------------------------------------------------------------------

/// A named extraction pattern with an optional `$1`/`$2`/`$0` substitution.
#[derive(Clone, Debug)]
pub struct UrlPattern {
    pub name: String,
    pub regex: Regex,
    pub substitution: Option<String>,
    /// When true, the match only counts when it is followed by whitespace or
    /// the end of the text. The pattern itself already requires the match to
    /// begin at the start of the text or right after whitespace, so together
    /// the two checks make the match a standalone whitespace-delimited word.
    pub standalone: bool,
}

/// Extracts URLs from captured pane content.
///
/// The built-in patterns mirror the reference tmux plugin exactly,
/// including the substitutions that normalize Git SSH URLs, bare `www`
/// domains, IP addresses, and GitHub `user/repo` shorthand into full URLs.
///
/// Bare `user/repo` shorthand is only recognized when it is a standalone
/// whitespace-delimited word, so path and URL segments such as `config/rmux`
/// or the `com/user` inside `https://github.com/user/repo` are not matched;
/// quoting the shorthand (`'user/repo'` or `"user/repo"`) also works.
#[derive(Debug)]
pub struct Extractor {
    patterns: Vec<UrlPattern>,
}

impl Extractor {
    /// The six built-in patterns from the reference tmux plugin, in the same order.
    pub fn with_defaults() -> Result<Self> {
        let mut extractor = Self {
            patterns: Vec::new(),
        };
        for (name, pattern, substitution, standalone) in default_patterns() {
            extractor.push_pattern(name, pattern, substitution, standalone)?;
        }
        Ok(extractor)
    }

    /// Append a custom pattern, mirroring `@open-url-custom-pat` with the
    /// optional `@open-url-custom-sub` replacement. When the substitution is
    /// absent, the whole match is used (`$0`).
    pub fn with_custom(&mut self, pattern: &str, substitution: Option<&str>) -> Result<()> {
        self.push_pattern("custom", pattern, substitution, false)
    }
    fn push_pattern(
        &mut self,
        name: &str,
        pattern: &str,
        substitution: Option<&str>,
        standalone: bool,
    ) -> Result<()> {
        let regex = Regex::new(pattern).with_context(|| format!("compile url pattern {name:?}"))?;
        self.patterns.push(UrlPattern {
            name: name.to_owned(),
            regex,
            substitution: substitution.map(str::to_owned),
            standalone,
        });
        Ok(())
    }

    /// Extract every URL in the order it appears in the text, applying each
    /// pattern's substitution. Matches contained in an earlier match (for
    /// example `www.example.com` inside `https://www.example.com`) are
    /// dropped, and repeated URLs — including duplicates produced by
    /// different patterns, such as a Git SSH URL and the literal https URL it
    /// normalizes to — are listed only once. A trailing `.git` is stripped
    /// from GitHub URLs so clone and web URLs deduplicate.
    pub fn extract(&self, text: &str) -> Vec<String> {
        let mut found: Vec<(usize, usize, String)> = Vec::new();
        for pattern in &self.patterns {
            for matched in pattern.regex.find_iter(text) {
                if pattern.standalone {
                    // The leading whitespace was consumed by the pattern;
                    // enforce the trailing whitespace here since the regex
                    // crate does not support look-ahead.
                    let end = matched.end();
                    if end < text.len()
                        && !text[end..].chars().next().is_some_and(char::is_whitespace)
                    {
                        continue;
                    }
                }
                let url = match &pattern.substitution {
                    Some(substitution) => pattern
                        .regex
                        .replace(matched.as_str(), substitution)
                        .into_owned(),
                    None => matched.as_str().to_owned(),
                };
                found.push((matched.start(), matched.end(), strip_git_suffix(&url)));
            }
        }
        found.sort_by_key(|(start, _, _)| *start);

        let mut urls = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let mut last_end = 0usize;
        for (start, end, url) in found {
            if start < last_end {
                continue;
            }
            if !seen.insert(url.clone()) {
                // Already reported: the range is still consumed, so later
                // matches nested inside it are dropped too.
                last_end = end;
                continue;
            }
            last_end = end;
            urls.push(url);
        }
        urls
    }
}

/// The built-in patterns from the reference tmux plugin (`PAT_URL`, `PAT_GIT`,
/// `PAT_WWW`, `PAT_IP`, `PAT_GH`), with their substitutions. The bare
/// `user/repo` shorthand is a `standalone` pattern: it must be surrounded by
/// whitespace (or line boundaries) to count, enforced by the pattern's
/// leading `(?:^|\s)` and the trailing check in `Extractor::extract`.
fn default_patterns() -> [(&'static str, &'static str, Option<&'static str>, bool); 6] {
    [
        (
            "url",
            r"(?:https?|ftp|file):/?//[-\w+&@#/%?=~|!:,.;]*[-\w+&@#/%=~|]",
            None,
            false,
        ),
        (
            "git",
            r#"(?:ssh://)?git@([^\s'"`:]+)[:/]([^\s'"`]+)"#,
            Some("https://$1/$2"),
            false,
        ),
        (
            "www",
            r#"www\.[a-zA-Z](?:-?[a-zA-Z0-9])+\.[a-zA-Z]{2,}(?:/[^\s'"`]+)*"#,
            Some("http://$0"),
            false,
        ),
        (
            "ip",
            r#"\d{1,3}\.\d{1,3}\.\d{1,3}\.\d{1,3}(?::\d{1,5})?(?:/[^\s'"`]+)*"#,
            Some("http://$0"),
            false,
        ),
        (
            "github",
            r#"['"]([\w-]+/[\w.-]+)['"]"#,
            Some("https://github.com/$1"),
            false,
        ),
        (
            "github-bare",
            r"(?:^|\s)([\w-]+/[\w-]+(?:\.[\w-]+)*)",
            Some("https://github.com/$1"),
            true,
        ),
    ]
}

/// Strip a trailing `.git` from GitHub URLs.
///
/// `https://github.com/owner/repo.git` and `https://github.com/owner/repo`
/// point at the same repository, so the clone-style suffix is removed to keep
/// the picker concise and to let clone and web URLs deduplicate. Only GitHub
/// hosts are affected.
fn strip_git_suffix(url: &str) -> String {
    for prefix in ["https://github.com/", "http://github.com/"] {
        if let Some(rest) = url.strip_prefix(prefix) {
            if let Some(repo) = rest.strip_suffix(".git") {
                return format!("{prefix}{repo}");
            }
            break;
        }
    }
    url.to_owned()
}

// ---------------------------------------------------------------------------
// ANSI stripping
// ---------------------------------------------------------------------------

/// Strip ANSI escape sequences (SGR colors, OSC window titles, charset
/// selects) from captured pane content so URLs are not split by styling.
pub fn strip_ansi(text: &str) -> String {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(
            r"\x1b\[[0-9;:?]*[ -/]*[@-~]|\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)|\x1b[()#][0-9A-Za-z]|\x1b",
        )
        .expect("ansi regex must compile")
    });
    re.replace_all(text, "").into_owned()
}

// ---------------------------------------------------------------------------
// Fuzzy filtering
// ---------------------------------------------------------------------------

/// Subsequence score for a query against a URL.
///
/// Returns `None` when `query` is not a subsequence of `text` (matched
/// case-insensitively). Matches at the start of the string, after a
/// separator, or in a consecutive run score higher; matches that appear
/// further down the string are penalized, keeping the most relevant URLs on
/// top.
pub fn fuzzy_score(query: &str, text: &str) -> Option<i64> {
    let query = query.to_lowercase().chars().collect::<Vec<_>>();
    if query.is_empty() {
        return Some(0);
    }
    let lower = text.to_lowercase().chars().collect::<Vec<_>>();
    let mut qi = 0usize;
    let mut score = 0i64;
    let mut consecutive = 0i64;
    let mut last: Option<usize> = None;
    for (idx, &ch) in lower.iter().enumerate() {
        if ch != query[qi] {
            continue;
        }
        qi += 1;
        let mut bonus = 1i64;
        if idx == 0 {
            bonus += 9;
        } else if is_separator(lower[idx - 1]) {
            bonus += 6;
        }
        if last == Some(idx - 1) {
            consecutive += 1;
            bonus += 3 * consecutive;
        } else {
            consecutive = 0;
        }
        score += bonus;
        last = Some(idx);
        if qi == query.len() {
            break;
        }
    }
    (qi == query.len()).then(|| score - last.unwrap_or(0) as i64 / 8)
}

fn is_separator(ch: char) -> bool {
    matches!(
        ch,
        ' ' | '_' | '-' | '/' | '.' | ':' | '@' | '~' | '=' | '?' | '&' | '#' | '+'
    )
}

/// Return candidate indices ordered by fuzzy score (best first), falling back
/// to original order on ties. An empty query returns every index in order.
pub fn filter_candidates(query: &str, urls: &[String]) -> Vec<usize> {
    let mut scored: Vec<(i64, usize)> = urls
        .iter()
        .enumerate()
        .filter_map(|(index, url)| fuzzy_score(query, url).map(|score| (score, index)))
        .collect();
    scored.sort_by(|left, right| right.0.cmp(&left.0).then(left.1.cmp(&right.1)));
    scored.into_iter().map(|(_, index)| index).collect()
}

// ---------------------------------------------------------------------------
// Actions: opening URLs and copying them to the clipboard
// ---------------------------------------------------------------------------

/// A parsed action: program, arguments, and an optional stdin payload.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActionInput {
    pub program: String,
    pub args: Vec<String>,
    pub stdin: Option<String>,
}

/// Parse a copy-style action. An argument equal to `{}` is replaced with
/// `selection`; otherwise the selection is written to the command's stdin.
pub fn build_copy_action(action: &str, selection: &str) -> Result<ActionInput> {
    let mut words = shell_words::split(action).context("parse copy action")?;
    if words.is_empty() {
        bail!("copy command must not be empty");
    }
    let program = words.remove(0);
    if let Some(position) = words.iter().position(|word| word == "{}") {
        words[position] = selection.to_owned();
        Ok(ActionInput {
            program,
            args: words,
            stdin: None,
        })
    } else {
        Ok(ActionInput {
            program,
            args: words,
            stdin: Some(selection.to_owned()),
        })
    }
}

/// Parse an open-style action. An argument equal to `{}` is replaced with
/// `url`; otherwise the URL is appended as an argument, since openers take
/// URLs on the command line rather than on stdin.
pub fn build_open_action(action: &str, url: &str) -> Result<ActionInput> {
    let mut words = shell_words::split(action).context("parse open action")?;
    if words.is_empty() {
        bail!("open command must not be empty");
    }
    let program = words.remove(0);
    if let Some(position) = words.iter().position(|word| word == "{}") {
        words[position] = url.to_owned();
    } else {
        words.push(url.to_owned());
    }
    Ok(ActionInput {
        program,
        args: words,
        stdin: None,
    })
}

/// Run an action and wait for it to finish, feeding `stdin` when present.
/// Used for clipboard copies so the popup can show a result.
pub fn run_action(input: &ActionInput) -> Result<()> {
    let mut command = Command::new(&input.program);
    command.args(&input.args);
    if input.stdin.is_some() {
        command.stdin(Stdio::piped());
    }
    let mut child = command
        .spawn()
        .with_context(|| format!("run action {:?}", input.program))?;
    if let Some(stdin) = input.stdin.as_deref() {
        child
            .stdin
            .take()
            .ok_or_else(|| anyhow!("action stdin was not available"))?
            .write_all(stdin.as_bytes())
            .context("write action input")?;
    }
    let status = child.wait().context("wait for action")?;
    if !status.success() {
        bail!("action exited with {status}");
    }
    Ok(())
}

/// Spawn an action detached from the popup so it survives the popup closing.
///
/// The child is placed in its own process group with stdio redirected, so the
/// browser opener keeps running after display-popup tears the popup pty down.
pub fn spawn_detached(input: &ActionInput) -> Result<()> {
    let mut command = Command::new(&input.program);
    command
        .args(&input.args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    command
        .spawn()
        .with_context(|| format!("spawn {:?}", input.program))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Opener and clipboard resolution
// ---------------------------------------------------------------------------

/// True when the current process runs under WSL.
pub fn is_wsl() -> bool {
    std::env::var_os("WSL_DISTRO_NAME").is_some() || std::env::var_os("WSL_INTEROP").is_some()
}

/// True when `program` resolves on PATH (or is a path to an existing file).
pub fn command_exists(program: &str) -> bool {
    if program.contains(std::path::MAIN_SEPARATOR) {
        return std::path::Path::new(program).is_file();
    }
    std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).any(|dir| dir.join(program).is_file()))
        .unwrap_or(false)
}

/// Resolve the URL opener: a custom
/// command wins, then WSL, `xdg-open`, `open`, and finally `$BROWSER`.
pub fn resolve_opener(custom: Option<&str>) -> Result<String> {
    if let Some(custom) = custom.filter(|value| !value.trim().is_empty()) {
        return Ok(custom.to_owned());
    }
    if is_wsl() {
        if command_exists("wslview") {
            return Ok("wslview".to_owned());
        }
        if command_exists("explorer.exe") {
            return Ok("explorer.exe".to_owned());
        }
    }
    for candidate in ["xdg-open", "open"] {
        if command_exists(candidate) {
            return Ok(candidate.to_owned());
        }
    }
    if let Some(browser) = std::env::var_os("BROWSER").filter(|value| !value.is_empty()) {
        return Ok(browser.to_string_lossy().into_owned());
    }
    bail!("no URL opener found: set @open-url-open or the BROWSER environment variable")
}

/// Resolve the clipboard command.
/// Falls back to `rmux load-buffer -` so copied URLs land in an rmux buffer.
pub fn resolve_copy_command(custom: Option<&str>, rmux: &str) -> String {
    if let Some(custom) = custom.filter(|value| !value.trim().is_empty()) {
        return custom.to_owned();
    }
    if is_wsl() && command_exists("clip.exe") {
        return "clip.exe".to_owned();
    }
    if command_exists("pbcopy") {
        return "pbcopy".to_owned();
    }
    if std::env::var_os("WAYLAND_DISPLAY").is_some() && command_exists("wl-copy") {
        return "wl-copy".to_owned();
    }
    if std::env::var_os("DISPLAY").is_some() {
        if command_exists("xclip") {
            return "xclip -selection clipboard".to_owned();
        }
        if command_exists("xsel") {
            return "xsel --clipboard --input".to_owned();
        }
    }
    format!("{} load-buffer -", shell_words::quote(rmux))
}

// ---------------------------------------------------------------------------
// rmux option parsing
// ---------------------------------------------------------------------------

/// Parse the output of `show-options -g` into `(name, value)` pairs.
///
/// This mirrors how the reference tmux plugin reads its options: tmux and rmux
/// print option values as tmux-quoted strings (double-quoted when they
/// contain spaces, with backslashes and quotes escaped), and this undoes that
/// quoting so the stored value is recovered.
pub fn parse_show_options(output: &str) -> Vec<(String, String)> {
    output
        .lines()
        .filter_map(|line| {
            let (name, value) = match line.split_once(' ') {
                Some((name, value)) => (name.trim(), value.trim_start()),
                None => (line.trim(), ""),
            };
            if name.is_empty() {
                return None;
            }
            Some((name.to_owned(), unquote_option(value)))
        })
        .collect()
}

/// Unquote a single option value as printed by `show-options -g`.
fn unquote_option(value: &str) -> String {
    if value.is_empty() {
        return String::new();
    }

    if value.starts_with('\'') && value.ends_with('\'') {
        // Single-quoted: swap quotes so the inner content can be parsed as a
        // double-quoted literal, then swap back.
        let inverted = invert_quotes(value);
        return invert_quotes(&unquote_double(&inverted));
    }

    let quoted = if value.starts_with('"') && value.ends_with('"') {
        value.to_owned()
    } else if !value.contains('"') {
        // Unquoted values still escape backslashes; wrap them so they go
        // through the same unescaping.
        format!("\"{value}\"")
    } else {
        return value.to_owned();
    };
    unquote_double(&quoted)
}

/// Parse a double-quoted string literal, undoing tmux/rmux escaping.
fn unquote_double(value: &str) -> String {
    let Some(inner) = value.strip_prefix('"').and_then(|v| v.strip_suffix('"')) else {
        return value.to_owned();
    };

    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        match chars.next() {
            Some('\\') => out.push('\\'),
            Some('"') => out.push('"'),
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            // Unknown escapes (\d, \s, \b, ...) are preserved literally so
            // regex patterns survive unchanged.
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

fn invert_quotes(value: &str) -> String {
    value
        .chars()
        .map(|ch| match ch {
            '\'' => '"',
            '"' => '\'',
            ch => ch,
        })
        .collect()
}
