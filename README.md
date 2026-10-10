# tubetui

YouTube in your terminal, written in Rust with [ratatui](https://ratatui.rs).
Videos play through [mpv](https://mpv.io) — embedded in the app (kitty
graphics protocol) or in their own window — one tab per video.

> **Status: early development.** Interface in Spanish for now.

## Features

- Sections: Inicio, Descubrir (home feed without your channels), Suscripciones,
  Ver después, Mis canales, search videos (all YouTube / my channels / inside a
  channel) and channels
- Subscribe / unsubscribe, watch later, download with subtitles
- Tabs for open videos, collapsible sidebar, volume / mute / subtitles
- Login from any installed browser (Brave, Chrome, Chromium, Vivaldi, Edge,
  Opera, Firefox) or an imported `cookies.txt` for machines without a browser
- Settings screen; everything is saved to `config.toml`, which keeps your comments
- Context hints at the bottom, all shortcuts with `Ctrl+y ?`

## Requirements

`mpv` and `yt-dlp` in `PATH`. For embedded video, the
[kitty](https://sw.kovidgoyal.net/kitty/) terminal; elsewhere videos open in an
mpv window.

## Install

```sh
cargo install tubetui
```

## Keys

Global commands use a `Ctrl+y` prefix (tmux style): `Ctrl+y b` toggles the menu,
`n`/`p`/`0-9` switch tabs, `x` closes one, `/` searches, `?` shows every
shortcut, `q` quits. The bottom bar always lists the keys that work right now.

## Configuration

`~/.config/tubetui/config.toml` (or the platform equivalent). Every key is
optional; it can also be edited from the app (⚙ Configuración).

```toml
[session]
browser = "brave+GNOMEKEYRING:Default"  # yt-dlp --cookies-from-browser syntax
cookies_file = ""                       # imported cookies.txt, wins over browser

[player]
mode = "auto"          # auto | embedded | window
max_height = 720
prefer_codec = "avc1"  # H.264 decodes in hardware almost everywhere
language = "subtitles" # subtitles | original | dubbed
sub_lang = "es"

[ui]
icons = "emoji"        # emoji | nerd | unicode
```

## Disclaimer

tubetui talks to YouTube's unofficial internal API and uses your browser
session. That may go against YouTube's Terms of Service; use it at your own
risk. Cookies give access to your account: never share them.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT), at your option.
