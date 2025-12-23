# Bbiribarabu

[한국어 README](README.ko.md)
Bbiribarabu is a branch-scoped logbook for Git repositories. It keeps quick notes per branch, offers a split TUI with an embedded shell, and can capture voice notes via Whisper.

## Features

- Branch-scoped logs saved under `.bbiribarabu/logs`
- TUI with shell + log list + input panel
- CLI for add/list/voice logging
- Voice transcription with on-demand Whisper model download

## Requirements

- Rust toolchain (edition 2024)
- Run inside a Git repository (uses `git rev-parse` and `git branch --show-current`)
- Microphone device for voice features
- Network access on first voice use, unless `WHISPER_MODEL` is set

## Quick start

### TUI

```bash
cargo run
```

### CLI

```bash
cargo run -- add "Fix flaky tests"
cargo run -- list
cargo run -- voice --seconds 5
```

## TUI controls

- `Esc`: switch focus between terminal and log panel
- Log panel (normal mode)
  - `i`: new log
  - `e`: edit selected log
  - `d`: delete selected log (confirm with `y`/`n`)
  - `/`: search logs
  - `v`: voice log (press `v` again to stop; any other key cancels)
  - `q`: quit
  - Arrow keys / PageUp / PageDown: move selection
  - Left / Right / Home: horizontal log scroll

## Data storage

- Logs are saved per branch at `.bbiribarabu/logs/<branch>.json`
- Branch slashes are replaced with `__` to keep filenames safe

## Voice model

- The Whisper base model is downloaded to `models/ggml-base.bin` when missing
- Override the model path with `WHISPER_MODEL=/path/to/ggml-base.bin`

## License

MIT. See `LICENSE`.
