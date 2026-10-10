# Benchmark

tubetui against the script it replaces (**ytv**: bash + fzf + yt-dlp + mpv)
and two popular alternatives: **[ytfzf](https://github.com/pystardust/ytfzf)**
(shell + fzf) and **[youtube-tui](https://github.com/siriusmart/youtube-tui)**
(Rust).

Measured on 2026-10-10 on a ThinkPad T430 (i5-3320M, 2 cores / 4 threads,
Intel HD 4000), Linux, kitty, mpv 0.41, yt-dlp 2026.08.19. Each app ran in its
own kitty window driven by remote control (keys in, screen text out); mpv was
muted and measured through its IPC socket. Search query: `ratatui rust
terminal`. Medians of 3 runs; playback measured once, 20 s after it started.
About 50 % of one core was busy with other programs during all runs.

| | ytv | tubetui (embedded) | tubetui (window) | ytfzf | youtube-tui |
|---|---|---|---|---|---|
| Start, menu ready ¹ | 0.04 s | 0.01 s | 0.01 s | — | — |
| Search ² | 4.8 s (60) | 4.5 s (60) | 4.4 s (60) | 1.5 s ³ (~20) | 1.2 s ³ (20) |
| Memory, idle (PSS) | 30 MB | 25 MB | 25 MB | 29 MB | 38–46 MB |
| CPU, idle | 0 % | 0 % | 0 % | 0 % | 0 % |
| Enter → video playing | 12.6 s | 12.0 s | 12.4 s | 3.1 s | 7.1 s |
| Video played | AV1 720p | H.264 720p | H.264 720p | AV1 1080p | AV1 1080p |
| Decoding | software | software | **hardware (vaapi)** | software | software |
| CPU while playing ⁴ | 21 % | 134 % + 8 % kitty | **8 %** | 79 % | 38 % |
| Dropped frames (35 s) | 1 | 405 | 0 | 144 | 0 |
| Audio / video sync | in sync | in sync | in sync | in sync | in sync |
| Video inside the terminal | no | **yes** | no | no | no |
| Tabs, background audio | no | **yes** | **yes** | no | no |
| Size | 252 lines bash | 6.5 MB binary | | 3 791 lines sh | 19 MB binary |
| Rust crates | — | 135 | | — | 282 |

¹ After kitty itself is up (1.78 s on this machine, the same for all).
² From Enter to the results on screen; in parentheses, how many results.
³ Includes starting the program: both take the query on the command line.
⁴ The app plus mpv, as a share of one core.

### After the fixes (tubetui 0.2.0, same machine, same day)

| | before | after |
|---|---|---|
| Search | 4.5 s (60 via yt-dlp) | **1.25 s** (20 per page, 20 more while scrolling) |
| Enter → video playing | 12 s | **≈8 s** (no cookies unless the video needs them) |
| Embedded 720p60, picture behind the sound | up to 97 s | **0.00002 s** |

## What it says

- **tubetui is as light as the bash script** when idle (25 MB, no CPU) and
  starts instantly.
- **Window mode is the most efficient player of all** (8 % CPU, no dropped
  frames): tubetui asks for H.264, which this GPU decodes.
- **Embedded video costs CPU**: every frame is scaled and sent to kitty as an
  image. At 720p this laptop drops about 40 % of the frames. 480p keeps
  every frame (measured earlier: one dropped frame in 20 s).
- **Search is slower than ytfzf and youtube-tui**: tubetui and ytv start
  yt-dlp (a Python program) and ask for 60 videos; the other two talk to
  YouTube directly and show 20.
- **Starting a video takes 12 s in tubetui and ytv**, against 3–7 s. Both pass
  cookies, a format choice and subtitle options to yt-dlp; which of those adds
  the time is still to be measured.

## What changed

1. Searches go to YouTube's internal API directly: 20 results a page, the
   next page loads while you scroll. The selected video shows its thumbnail.
2. Videos open without the login cookies: with them yt-dlp took 23 s against
   7 s. A video that needs the login (age-restricted, members-only) is retried
   with it automatically.
3. Embedded video prefers 30 fps and lets the decoder skip frames when the CPU
   falls behind, so the picture never drifts from the sound.

Still to do: 480p by default for embedded video.
