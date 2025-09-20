# rlless

`rlless` is a fast, asynchronous log viewer inspired by `less`, built in Rust. It streams
through multi-gigabyte files, coalesces terminal input, and coordinates a search worker
with a render loop to keep the UI responsive.

## Features

- Streaming viewport powered by asynchronous file accessors (memory-mapped or adaptive).
- `less`-style navigation (`j`/`k`, PgUp/PgDn, `g`/`G`, `/` / `?` searches).
- Runtime search toggles via command mode (`-i`, `-r`, `-n`, `-w`).
- Percent-based jumps with `%NN` syntax.
- Search history recall inside the prompt (arrow keys to cycle).

## Installation

```bash
cargo build --release
```
The binary will be produced at `target/release/rlless`.

## Usage

```bash
cargo run -- <path-to-log>
```

### Navigation

- `j` / `Down` – scroll down one line
- `k` / `Up` – scroll up one line
- `Space`, `PgDn`, `f` – page down
- `PgUp`, `b` – page up
- `/` – enter forward search prompt
- `?` – enter backward search prompt
- `%` – enter percentage jump prompt (type a number, `Enter` to jump)
- `-` – enter command mode for toggles (`i` case-sensitivity, `r` regex, `n` literal, `w` whole word)
- `q` – quit

### Search Prompt Shortcuts

- `Enter` – execute search with current buffer
- `Esc` / `Ctrl+C` – exit search mode
- `Up` / `Down` – recall previous search patterns (edit in place)

### Percent Jump Prompt

- Type a number (0–100) and press `Enter` to jump to that percentage
- `Esc`, `Ctrl+C`, or backspace on an empty buffer cancels

## Development

- `cargo fmt` – format the codebase
- `cargo clippy --all-targets --all-features` – lint
- `cargo test --all --all-features` – run tests
- `cargo run -- <log>` – launch the TUI

## Contributing

See `AGENTS.md` and design documents under `design/` for architectural notes.
