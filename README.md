# rmux-open-url

`rmux-open-url` is a tool for opening URL links found in the screen content
of your [rmux](https://rmux.io/docs/get-started/) panes.

It captures the visible content of the current pane, extracts the URL links on
screen, and presents them in an interactive list for one-key opening in the
browser. Duplicate URLs are automatically deduplicated, so each link —
including the same link produced by different patterns, such as a Git SSH
address and the https address it converts to — is listed only once:

- **Enter** opens the selected URLs in your browser.
- **Tab** toggles multi-select; with nothing selected, Enter opens every
  filtered match.
- **Ctrl-y** copies the selected URLs (or the current line) to the clipboard
  and keeps the picker open.
- **Esc** or **Ctrl-c** cancels.

The URL patterns are identical to the reference plugin:

- Standard URLs — `https://`, `http://`, `ftp://`, `file://`
- Git SSH URLs — `git@github.com:user/repo` and `ssh://git@github.com/user/repo`
  (normalized to `https://github.com/...`, with any trailing `.git` removed)
- Bare `www` domains — `www.example.com` (prefixed with `http://`)
- IP addresses — `192.168.1.1`, `10.0.0.1:8080/path` (prefixed with `http://`)
- GitHub shorthand — `user/repo` as a standalone whitespace-delimited word
  (one slash between two words, converted to `https://github.com/user/repo`);
  quoted `'user/repo'` or `"user/repo"` is also recognized
- Custom patterns — via `@open-url-custom-pat` / `@open-url-custom-sub`

## Build and install

```sh
make check
make install
```

`make install` builds an optimized release binary and installs it as
`~/.cargo/bin/rmux-open-url`. Run `make help` to list the other build targets.

## Configure rmux

The picker runs in a `display-popup` overlay and never changes windows, panes,
or their layout. The default keybinding is `prefix + u`, matching
`set -g @open-url-bind 'u'` from the reference `~/.tmux.conf`:

```tmux
bind u display-popup -B -E -x center -y center -w 100% -h 50% -d "#{pane_current_path}" "$HOME/.cargo/bin/rmux-open-url --pane '#{pane_id}'"
```

Change `u` to any other key by editing the `bind u` line.

## Use

1. Press `prefix + u`.
2. Type to filter the URLs (fuzzy matching, case-insensitive).
3. Move with arrows, `Ctrl-j/k`, `Ctrl-n/p`, or `Ctrl-a/e`; toggle selection
   with `Tab`; `Ctrl-u` clears the query.
4. Press `Enter` to open the URLs, or `Ctrl-y` to copy them instead.
5. Press `Esc` or `Ctrl-c` to cancel.

## Customize

Everything is configurable through `@open-url-*` options in
`~/.config/rmux/rmux.conf`, exactly like the reference plugin; command line
flags always take precedence.

Capture scrollback history instead of just the visible screen:

```tmux
set -g @open-url-history-limit '2000'
```

Use a custom opener (the URL is appended as an argument, or replaces `{}`):

```tmux
set -g @open-url-open "firefox"
set -g @open-url-open "open -a 'Google Chrome' {}"
```

Use a custom clipboard command (URLs are joined with newlines and written to
stdin, or replace `{}`):

```tmux
set -g @open-url-copy-cmd 'xclip -selection clipboard'
```

Add a custom extraction pattern and optional replacement (`$0` is the whole
match):

```tmux
set -g @open-url-custom-pat '\b[a-zA-Z]+\.txt\b'
set -g @open-url-custom-pat '[A-Z]+-\d+'
set -g @open-url-custom-sub 'https://jira.example.com/browse/$0'
```

Backslashes must be doubled inside the double quotes, exactly as in tmux and
rmux config files. Reload the config (`bind r` sources
`~/.config/rmux/rmux.conf` in the default setup) or restart the server after
editing.

By default the opener is detected as in the reference plugin — `wslview`/
`explorer.exe` on WSL, `xdg-open` on Linux, `open` on macOS, then `$BROWSER` —
and the clipboard tool is auto-detected too (`clip.exe`, `pbcopy`, `wl-copy`,
`xclip`/`xsel`, falling back to `rmux load-buffer -`).

Note: the picker is a native terminal UI, so there is no external selector
process to configure.

Run `rmux-open-url --help` for the complete CLI reference.

See [CONTRIBUTING.md](CONTRIBUTING.md) if you'd like to help improve the project.
