# Contributing to rmux-open-url

Thanks for your interest in contributing! This project follows the conventions
of the surrounding [rmux](https://rmux.io/docs/get-started/) ecosystem and of
the original tmux plugin it reimplements.

## Ways to contribute

- **Report bugs** — open an issue with the reproduction steps, the
  `rmux-open-url` version (`rmux-open-url --version`), and your rmux/terminal
  setup.
- **Suggest features** — open an issue describing the use case before writing
  code.
- **Fix bugs and add features** — fork the repository, make your change, and
  open a pull request.

## Prerequisites

- Rust toolchain (edition 2024 is used, so a recent stable Rust is required)
- `cargo` and the usual Rust tooling (`rustfmt`, `clippy`)
- [rmux](https://rmux.io/docs/get-started/) for manual end-to-end testing

## Project layout

```
src/
  lib.rs          Core logic: URL extraction, fuzzy filtering, actions
  main.rs         CLI entry point, pane capture, and the interactive picker
tests/
  core.rs         Unit and integration tests for extraction and actions
  layout_safety.rs  Tests asserting the picker never changes the layout
```

## Development workflow

Clone the repository and build:

```sh
git clone https://github.com/tenfyzhong/rmux-open-url
cd rmux-open-url
cargo build
```

Run the full check suite (formatting, lint, and tests):

```sh
make check
```

Or run the steps individually:

```sh
make fmt        # auto-format sources
make fmt-check  # verify formatting
make lint       # clippy with -D warnings
make test       # cargo test
make build      # debug build
make release    # optimized release build
```

## Design notes

The URL patterns and their substitutions, the opener and clipboard resolution,
and the `@open-url-*` option names all mirror the reference tmux plugin so
existing configs keep working. The interactive picker is implemented directly
with crossterm inside a `display-popup` overlay, so there is no external
selector process to configure.

The picker only reads the target pane with `capture-pane` and writes to an
rmux buffer or the system clipboard. It must never swap, resize, split, or
otherwise change windows, panes, or their layout — `tests/layout_safety.rs`
guards against this.
