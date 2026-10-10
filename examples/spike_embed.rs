//! Phase 0 spike: can mpv draw video inside a ratatui pane through the kitty
//! graphics protocol while ratatui owns the rest of the screen?
//!
//! Run inside kitty:
//!   cargo run --release --example spike_embed -- <url> [height] [embedded|window]
//!
//! Keys: `Ctrl+y b` toggle sidebar · `space` pause · `q` quit.
//! Throwaway code: findings go into the real `player` module.

use std::io::Write;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::widgets::{Block, List, Paragraph};
use ratatui::{DefaultTerminal, Frame};

const SIDEBAR_WIDTH: u16 = 24;
const PREFIX_TIMEOUT: Duration = Duration::from_millis(1500);

struct Mpv {
    child: Child,
    socket: PathBuf,
    ipc: Option<UnixStream>,
}

impl Mpv {
    fn spawn(url: &str, height: u32, embed: Option<Rect>) -> std::io::Result<Self> {
        let socket =
            std::env::temp_dir().join(format!("tubetui-spike-{}.sock", std::process::id()));
        let mut cmd = Command::new("mpv");
        cmd.arg(format!("--input-ipc-server={}", socket.display()))
            .arg(format!(
                "--ytdl-format=bv*[height<={height}]+ba/b[height<={height}]"
            ))
            .args(["--really-quiet", "--input-terminal=no"])
            .args(
                std::env::var("SPIKE_MPV_ARGS")
                    .unwrap_or_default()
                    .split_whitespace(),
            )
            .arg(url)
            .stdin(Stdio::null())
            .stderr(Stdio::null());
        match embed {
            Some(area) => {
                cmd.args(["--vo=kitty", "--vo-kitty-use-shm=yes"])
                    .args(["--vo-kitty-alt-screen=no", "--vo-kitty-config-clear=no"])
                    .args(kitty_geometry(area));
            }
            None => {
                cmd.args(["--hwdec=auto", "--force-window=immediate"])
                    .stdout(Stdio::null());
            }
        }
        Ok(Self {
            child: cmd.spawn()?,
            socket,
            ipc: None,
        })
    }

    fn command(&mut self, json: &str) {
        if self.ipc.is_none() {
            self.ipc = UnixStream::connect(&self.socket).ok();
        }
        if let Some(ipc) = self.ipc.as_mut()
            && writeln!(ipc, "{json}").is_err()
        {
            self.ipc = None;
        }
    }

    /// Moves the embedded video: update the kitty options, then recreate the
    /// video output by toggling the video track (keeps playback position).
    fn move_to(&mut self, area: Rect) {
        for (name, value) in kitty_values(area) {
            self.command(&format!(
                r#"{{"command":["set_property","options/vo-kitty-{name}",{value}]}}"#
            ));
        }
        self.command(r#"{"command":["set_property","vid","no"]}"#);
        self.command(r#"{"command":["set_property","vid","auto"]}"#);
    }
}

impl Drop for Mpv {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_file(&self.socket);
    }
}

/// mpv positions are 1-based cells; it also needs the pane size in pixels,
/// otherwise it scales the video to the whole terminal.
fn kitty_values(area: Rect) -> [(&'static str, u16); 6] {
    let (cell_w, cell_h) = cell_pixels();
    [
        ("left", area.x + 1),
        ("top", area.y + 1),
        ("cols", area.width),
        ("rows", area.height),
        ("width", area.width * cell_w),
        ("height", area.height * cell_h),
    ]
}

fn cell_pixels() -> (u16, u16) {
    match ratatui::crossterm::terminal::window_size() {
        Ok(ws) if ws.columns > 0 && ws.rows > 0 && ws.width > 0 => {
            (ws.width / ws.columns, ws.height / ws.rows)
        }
        _ => (10, 20),
    }
}

fn kitty_geometry(area: Rect) -> Vec<String> {
    kitty_values(area)
        .iter()
        .map(|(name, value)| format!("--vo-kitty-{name}={value}"))
        .collect()
}

struct App {
    sidebar: bool,
    prefix_at: Option<Instant>,
    video_area: Rect,
    status: String,
}

fn draw(frame: &mut Frame, app: &mut App) {
    let [body, status] =
        Layout::vertical([Constraint::Fill(1), Constraint::Length(1)]).areas(frame.area());
    let sidebar_width = if app.sidebar { SIDEBAR_WIDTH } else { 0 };
    let [side, main] =
        Layout::horizontal([Constraint::Length(sidebar_width), Constraint::Fill(1)]).areas(body);
    if app.sidebar {
        let items = [
            "Feed",
            "Suscripciones",
            "Ver después",
            "Descubrir",
            "Historial",
            "Ajustes",
        ];
        frame.render_widget(
            List::new(items).block(Block::bordered().title(" tubetui ")),
            side,
        );
    }
    let video = Block::bordered().title(" video ");
    app.video_area = video.inner(main);
    frame.render_widget(video, main);
    let prefix = if app.prefix_at.is_some() {
        "[C-y] "
    } else {
        ""
    };
    frame.render_widget(Paragraph::new(format!("{prefix}{}", app.status)), status);
}

fn run(
    terminal: &mut DefaultTerminal,
    url: &str,
    height: u32,
    embedded: bool,
) -> std::io::Result<()> {
    let mut app = App {
        sidebar: true,
        prefix_at: None,
        video_area: Rect::default(),
        status: String::new(),
    };
    app.status = format!(
        "{} {height}p · C-y b menú · espacio pausa · q salir",
        if embedded { "embedded" } else { "window" }
    );
    terminal.draw(|f| draw(f, &mut app))?;
    let mut mpv = Mpv::spawn(url, height, embedded.then_some(app.video_area))?;

    loop {
        if app.prefix_at.is_some_and(|t| t.elapsed() > PREFIX_TIMEOUT) {
            app.prefix_at = None;
            terminal.draw(|f| draw(f, &mut app))?;
        }
        if mpv.child.try_wait()?.is_some() {
            return Ok(());
        }
        if !event::poll(Duration::from_millis(100))? {
            continue;
        }
        let before = app.video_area;
        match event::read()? {
            Event::Key(KeyEvent {
                code: KeyCode::Char('y'),
                modifiers: KeyModifiers::CONTROL,
                ..
            }) => {
                app.prefix_at = Some(Instant::now());
            }
            // Any key after the prefix is consumed; only `b` does something yet.
            Event::Key(key) if app.prefix_at.take().is_some() => {
                app.sidebar ^= key.code == KeyCode::Char('b')
            }
            Event::Key(key) if key.code == KeyCode::Char('q') => return Ok(()),
            Event::Key(key) if key.code == KeyCode::Char(' ') => {
                mpv.command(r#"{"command":["cycle","pause"]}"#)
            }
            Event::Resize(..) => terminal.clear()?,
            _ => {}
        }
        terminal.draw(|f| draw(f, &mut app))?;
        if embedded && app.video_area != before {
            mpv.move_to(app.video_area);
        }
    }
}

fn main() -> std::io::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let url = args
        .get(1)
        .map(String::as_str)
        .unwrap_or("https://www.youtube.com/watch?v=aqz-KE-bpKQ");
    let height = args.get(2).and_then(|h| h.parse().ok()).unwrap_or(480);
    let embedded = args.get(3).is_none_or(|m| m != "window");
    let mut terminal = ratatui::init();
    let result = run(&mut terminal, url, height, embedded);
    ratatui::restore();
    result
}
