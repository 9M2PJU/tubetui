# tubetui

YouTube in your terminal, written in Rust with [ratatui](https://ratatui.rs).

> **Status: early development.** Nothing usable yet — this release reserves the name.

## Planned

- Search, subscriptions feed, Watch later, Discover
- Playback through [mpv](https://mpv.io): embedded in a pane (kitty graphics protocol) or in its own window
- Tabs for open videos, collapsible sidebar, `Ctrl+y` prefix keys (tmux/herdr style)
- Multiple accounts, with cookies read from your installed browsers
- Config in `config.toml`, editable from inside the app
- Linux, macOS and Windows

## Disclaimer

tubetui talks to YouTube's unofficial internal API and can use your browser
session. That may go against YouTube's Terms of Service; use it at your own risk.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT), at your option.
