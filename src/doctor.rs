//! What tubetui needs from the system (mpv, yt-dlp…), whether it is there,
//! and the command that installs it with this system's package manager.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::SystemTime;

use crate::i18n::Text;

/// yt-dlp older than this many days often fails: YouTube changes all the time.
const YT_DLP_MAX_AGE_DAYS: i64 = 90;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Manager {
    Apt,
    Pacman,
    Dnf,
    Zypper,
    Brew,
    Winget,
    Unknown,
}

impl Manager {
    pub fn name(self) -> &'static str {
        match self {
            Self::Apt => "apt",
            Self::Pacman => "pacman",
            Self::Dnf => "dnf",
            Self::Zypper => "zypper",
            Self::Brew => "Homebrew",
            Self::Winget => "winget",
            Self::Unknown => "?",
        }
    }
}

/// The package manager, from the OS and Linux's `/etc/os-release`.
pub fn manager(os: &str, os_release: &str) -> Manager {
    match os {
        "windows" => return Manager::Winget,
        "macos" => return Manager::Brew,
        _ => {}
    }
    let field = |key: &str| {
        os_release
            .lines()
            .find_map(|l| l.strip_prefix(key)?.strip_prefix('='))
            .map(|v| v.trim_matches('"').to_lowercase())
            .unwrap_or_default()
    };
    let ids = format!("{} {}", field("ID"), field("ID_LIKE"));
    let is = |names: &[&str]| ids.split_whitespace().any(|id| names.contains(&id));
    if is(&["debian", "ubuntu"]) {
        Manager::Apt
    } else if is(&["arch", "manjaro", "endeavouros"]) {
        Manager::Pacman
    } else if is(&["fedora", "rhel", "centos"]) {
        Manager::Dnf
    } else if is(&["opensuse", "suse", "opensuse-tumbleweed", "opensuse-leap"]) {
        Manager::Zypper
    } else {
        Manager::Unknown
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Mpv,
    YtDlp,
    Ffmpeg,
    Kitty,
    Xdotool,
    Xwininfo,
    Xprop,
    Wmctrl,
    /// The keyring the browsers' cookie key is in, and what yt-dlp needs to open it.
    Keyring,
}

impl Tool {
    pub fn program(self) -> &'static str {
        match self {
            Self::Mpv => "mpv",
            Self::YtDlp => "yt-dlp",
            Self::Ffmpeg => "ffmpeg",
            Self::Kitty => "kitty",
            Self::Xdotool => "xdotool",
            Self::Xwininfo => "xwininfo",
            Self::Xprop => "xprop",
            Self::Wmctrl => "wmctrl",
            Self::Keyring => "keyring",
        }
    }

    pub fn required(self) -> bool {
        matches!(self, Self::Mpv | Self::YtDlp)
    }

    /// What it is for.
    pub fn purpose(self) -> Text {
        match self {
            Self::Mpv => Text::new("reproduce los videos y la música", "plays videos and music"),
            Self::YtDlp => Text::new("lee YouTube: listas, sesión y videos", "reads YouTube: lists, login and videos"),
            Self::Ffmpeg => Text::new("descargas con video y audio juntos (d)", "downloads with video and sound (d)"),
            Self::Kitty => {
                Text::new("terminal con el video dentro de la app", "terminal with the video inside the app")
            }
            Self::Xdotool => Text::new("modo superpuesto (X11)", "overlay mode (X11)"),
            Self::Xwininfo => Text::new("modo superpuesto (X11)", "overlay mode (X11)"),
            Self::Xprop => Text::new("modo superpuesto (X11)", "overlay mode (X11)"),
            Self::Wmctrl => Text::new("pantalla completa de la terminal (X11)", "terminal fullscreen (X11)"),
            Self::Keyring => Text::new(
                "llavero: guarda la clave de la sesión de Chrome/Brave",
                "keyring: holds the key to Chrome/Brave's login",
            ),
        }
    }

    /// Package name for `manager`; `None` when it has none there.
    fn package(self, manager: Manager) -> Option<&'static str> {
        use Manager::*;
        Some(match (self, manager) {
            (_, Unknown) | (Self::Keyring, _) => return None,
            (Self::Mpv, Winget) => "shinchiro.mpv",
            (Self::YtDlp, Winget) => "yt-dlp.yt-dlp",
            (Self::Ffmpeg, Winget) => "Gyan.FFmpeg",
            (Self::Kitty | Self::Xdotool | Self::Xwininfo | Self::Xprop | Self::Wmctrl, Winget) => return None,
            (Self::Xdotool | Self::Xwininfo | Self::Xprop | Self::Wmctrl, Brew) => return None,
            (Self::Ffmpeg, Dnf) => "ffmpeg-free",
            (Self::Xwininfo | Self::Xprop, Apt) => "x11-utils",
            (Self::Xwininfo, Pacman) => "xorg-xwininfo",
            (Self::Xprop, Pacman) => "xorg-xprop",
            (tool, _) => tool.program(),
        })
    }
}

/// The tools worth checking here: the X11 helpers only where X11 is.
pub fn tools(os: &str) -> Vec<Tool> {
    let mut tools = vec![Tool::Mpv, Tool::YtDlp, Tool::Ffmpeg];
    if os != "windows" {
        tools.push(Tool::Kitty);
    }
    if os == "linux" {
        tools.extend([Tool::Xdotool, Tool::Xwininfo, Tool::Xprop, Tool::Wmctrl, Tool::Keyring]);
    }
    tools
}

/// The command that installs `tool`; `None` when the manager is unknown or
/// has no package for it.
pub fn install_command(tool: Tool, manager: Manager) -> Option<String> {
    // Distro packages of yt-dlp lag months behind YouTube; pipx keeps it current.
    if tool == Tool::YtDlp && manager == Manager::Apt {
        return Some("sudo apt install pipx && pipx install yt-dlp".into());
    }
    let package = tool.package(manager)?;
    Some(match manager {
        Manager::Apt => format!("sudo apt install {package}"),
        Manager::Pacman => format!("sudo pacman -S {package}"),
        Manager::Dnf => format!("sudo dnf install {package}"),
        Manager::Zypper => format!("sudo zypper install {package}"),
        Manager::Brew if tool == Tool::Kitty => "brew install --cask kitty".into(),
        Manager::Brew => format!("brew install {package}"),
        Manager::Winget => format!("winget install --id {package} -e"),
        Manager::Unknown => return None,
    })
}

/// How to bring an old yt-dlp up to date, depending on where it is.
pub fn update_yt_dlp(path: &str, manager: Manager) -> String {
    if path.contains("pipx") || path.contains("/.local/bin/") {
        "pipx upgrade yt-dlp".into()
    } else {
        match manager {
            Manager::Apt => "sudo apt install pipx && pipx install yt-dlp".into(),
            Manager::Pacman => "sudo pacman -Syu yt-dlp".into(),
            Manager::Dnf => "sudo dnf upgrade yt-dlp".into(),
            Manager::Zypper => "sudo zypper update yt-dlp".into(),
            Manager::Brew => "brew upgrade yt-dlp".into(),
            Manager::Winget => "winget upgrade --id yt-dlp.yt-dlp -e".into(),
            Manager::Unknown => "yt-dlp -U".into(),
        }
    }
}

/// How to give yt-dlp the `secretstorage` module it opens gnome-keyring with.
pub fn install_secretstorage(yt_dlp: &str, manager: Manager) -> String {
    if yt_dlp.contains("pipx") || yt_dlp.contains("/.local/bin/") {
        return "pipx inject yt-dlp secretstorage".into();
    }
    match manager {
        Manager::Apt => "sudo apt install python3-secretstorage".into(),
        Manager::Pacman => "sudo pacman -S python-secretstorage".into(),
        Manager::Dnf => "sudo dnf install python3-secretstorage".into(),
        Manager::Zypper => "sudo zypper install python3-SecretStorage".into(),
        _ => "pip install secretstorage".into(),
    }
}

/// One line of the check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Check {
    pub tool: Tool,
    /// Where it is, and its version when it tells.
    pub found: Option<(String, Option<String>)>,
    /// yt-dlp is there but too old; for the keyring, yt-dlp can't open it.
    pub outdated: bool,
    /// What to run: install it, or update it.
    pub fix: Option<String>,
}

impl Check {
    pub fn ok(&self) -> bool {
        self.found.is_some() && !self.outdated
    }
}

/// Looks for every tool on this machine (runs `--version` of the ones found).
pub fn run() -> (Manager, Vec<Check>) {
    let os = std::env::consts::OS;
    let release = std::fs::read_to_string("/etc/os-release").unwrap_or_default();
    let manager = manager(os, &release);
    let checks = tools(os)
        .into_iter()
        .map(|tool| check(tool, manager))
        // No keyring is fine: the browsers then keep their cookies unencrypted.
        .filter(|c| c.tool != Tool::Keyring || c.found.is_some())
        .collect();
    (manager, checks)
}

fn check(tool: Tool, manager: Manager) -> Check {
    if tool == Tool::Keyring {
        return check_keyring(manager);
    }
    let Some(path) = find(tool.program()) else {
        return Check { tool, found: None, outdated: false, fix: install_command(tool, manager) };
    };
    let path = path.display().to_string();
    let version = version_of(tool, &path);
    let outdated =
        tool == Tool::YtDlp && version.as_deref().is_some_and(|v| yt_dlp_age_days(v, today()) > YT_DLP_MAX_AGE_DAYS);
    let fix = outdated.then(|| update_yt_dlp(&path, manager));
    Check { tool, found: Some((path, version)), outdated, fix }
}

/// The keyring found (as yt-dlp names it), and whether yt-dlp can open it:
/// gnome-keyring needs its `secretstorage` module, KWallet `kwallet-query`.
fn check_keyring(manager: Manager) -> Check {
    let keyring = crate::session::detect_keyring();
    let mut check = Check { tool: Tool::Keyring, found: None, outdated: false, fix: None };
    if keyring.is_empty() {
        return check;
    }
    if keyring.starts_with("KWALLET") {
        check.outdated = find("kwallet-query").is_none();
    } else if let Some(yt_dlp) = find("yt-dlp") {
        let yt_dlp = yt_dlp.display().to_string();
        // `-v` lists the optional modules yt-dlp could load.
        let out = Command::new(&yt_dlp).arg("-v").stdin(Stdio::null()).output();
        let text = out.map(|o| String::from_utf8_lossy(&o.stderr).into_owned()).unwrap_or_default();
        if text.contains("Optional libraries") && !text.contains("secretstorage") {
            check.outdated = true;
            check.fix = Some(install_secretstorage(&yt_dlp, manager));
        }
    }
    check.found = Some((keyring.clone(), Some(keyring)));
    check
}

/// The program on `PATH` (with Windows' `.exe`).
pub fn find(program: &str) -> Option<PathBuf> {
    let paths = std::env::var_os("PATH")?;
    let names =
        if cfg!(windows) { vec![format!("{program}.exe"), program.to_owned()] } else { vec![program.to_owned()] };
    std::env::split_paths(&paths).flat_map(|dir| names.iter().map(move |n| dir.join(n))).find(|p| p.is_file())
}

fn version_of(tool: Tool, path: &str) -> Option<String> {
    let flag = if tool == Tool::Ffmpeg { "-version" } else { "--version" };
    if matches!(tool, Tool::Xdotool | Tool::Xwininfo | Tool::Xprop | Tool::Wmctrl) {
        return None; // there is nothing to compare
    }
    let out = Command::new(path).arg(flag).stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
    parse_version(tool, &String::from_utf8_lossy(&out.stdout))
}

/// `mpv v0.41.0 …` → `0.41.0`; `2026.08.19` → itself; `ffmpeg version 6.1 …` → `6.1`.
pub fn parse_version(tool: Tool, output: &str) -> Option<String> {
    let first = output.lines().next()?.trim();
    let word = match tool {
        Tool::YtDlp => first.split_whitespace().next(),
        Tool::Ffmpeg => first.split_whitespace().nth(2),
        _ => first.split_whitespace().nth(1),
    }?;
    Some(word.trim_start_matches('v').to_owned())
}

/// Days since a yt-dlp version (`2026.08.19`, maybe with a fourth part) came out.
pub fn yt_dlp_age_days(version: &str, today: (i64, i64, i64)) -> i64 {
    let mut parts = version.split('.').map(|p| p.parse::<i64>().unwrap_or(0));
    let date = (parts.next().unwrap_or(0), parts.next().unwrap_or(1), parts.next().unwrap_or(1));
    days(today) - days(date)
}

/// Rough day count, fine for "older than three months".
fn days((y, m, d): (i64, i64, i64)) -> i64 {
    y * 365 + (m - 1) * 30 + d
}

fn today() -> (i64, i64, i64) {
    let secs = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
    let (y, m, d) = crate::domain::civil_from_days(secs.div_euclid(86_400));
    (y, i64::from(m), i64::from(d))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn package_manager_from_the_system() {
        assert_eq!(manager("linux", "ID=ubuntu\nID_LIKE=debian\n"), Manager::Apt);
        assert_eq!(manager("linux", "ID=linuxmint\nID_LIKE=\"ubuntu debian\"\n"), Manager::Apt);
        assert_eq!(manager("linux", "ID=arch\n"), Manager::Pacman);
        assert_eq!(manager("linux", "ID=endeavouros\nID_LIKE=arch\n"), Manager::Pacman);
        assert_eq!(manager("linux", "ID=fedora\n"), Manager::Dnf);
        assert_eq!(manager("linux", "ID=\"opensuse-tumbleweed\"\nID_LIKE=\"opensuse suse\"\n"), Manager::Zypper);
        assert_eq!(manager("linux", "ID=gentoo\n"), Manager::Unknown);
        assert_eq!(manager("windows", ""), Manager::Winget);
        assert_eq!(manager("macos", ""), Manager::Brew);
    }

    #[test]
    fn install_commands_use_each_systems_names() {
        assert_eq!(install_command(Tool::Mpv, Manager::Apt).unwrap(), "sudo apt install mpv");
        assert_eq!(install_command(Tool::YtDlp, Manager::Apt).unwrap(), "sudo apt install pipx && pipx install yt-dlp");
        assert_eq!(install_command(Tool::Xprop, Manager::Apt).unwrap(), "sudo apt install x11-utils");
        assert_eq!(install_command(Tool::Xwininfo, Manager::Pacman).unwrap(), "sudo pacman -S xorg-xwininfo");
        assert_eq!(install_command(Tool::Ffmpeg, Manager::Dnf).unwrap(), "sudo dnf install ffmpeg-free");
        assert_eq!(install_command(Tool::Mpv, Manager::Winget).unwrap(), "winget install --id shinchiro.mpv -e");
        assert_eq!(install_command(Tool::Kitty, Manager::Brew).unwrap(), "brew install --cask kitty");
        assert_eq!(install_command(Tool::Kitty, Manager::Winget), None);
        assert_eq!(install_command(Tool::Mpv, Manager::Unknown), None);
    }

    #[test]
    fn tools_depend_on_the_system() {
        assert_eq!(tools("windows"), [Tool::Mpv, Tool::YtDlp, Tool::Ffmpeg]);
        assert!(tools("macos").contains(&Tool::Kitty) && !tools("macos").contains(&Tool::Xdotool));
        assert_eq!(tools("linux").len(), 9);
        assert!(tools("linux").contains(&Tool::Keyring) && !tools("macos").contains(&Tool::Keyring));
        assert!(Tool::Mpv.required() && Tool::YtDlp.required() && !Tool::Kitty.required());
    }

    #[test]
    fn secretstorage_goes_where_yt_dlp_is() {
        assert_eq!(
            install_secretstorage("/home/u/.local/bin/yt-dlp", Manager::Apt),
            "pipx inject yt-dlp secretstorage"
        );
        assert_eq!(install_secretstorage("/usr/bin/yt-dlp", Manager::Apt), "sudo apt install python3-secretstorage");
        assert_eq!(install_secretstorage("/usr/bin/yt-dlp", Manager::Pacman), "sudo pacman -S python-secretstorage");
    }

    #[test]
    fn versions_and_age() {
        assert_eq!(
            parse_version(Tool::Mpv, "mpv v0.41.0 Copyright © 2000-2025 mpv/MPlayer/mplayer2 projects\n").unwrap(),
            "0.41.0"
        );
        assert_eq!(parse_version(Tool::YtDlp, "2026.08.19\n").unwrap(), "2026.08.19");
        assert_eq!(parse_version(Tool::Ffmpeg, "ffmpeg version 6.1.1-3ubuntu5 Copyright\n").unwrap(), "6.1.1-3ubuntu5");
        assert_eq!(parse_version(Tool::Kitty, "kitty 0.39.1 created by Kovid Goyal\n").unwrap(), "0.39.1");
        assert_eq!(parse_version(Tool::Mpv, ""), None);
        assert_eq!(yt_dlp_age_days("2026.08.19", (2026, 10, 10)), 51);
        assert!(yt_dlp_age_days("2024.04.09", (2026, 10, 10)) > YT_DLP_MAX_AGE_DAYS);
        assert_eq!(update_yt_dlp("/home/u/.local/bin/yt-dlp", Manager::Apt), "pipx upgrade yt-dlp");
        assert_eq!(update_yt_dlp("/usr/bin/yt-dlp", Manager::Pacman), "sudo pacman -Syu yt-dlp");
    }

    #[test]
    fn a_check_is_ok_only_when_found_and_current() {
        let found =
            Check { tool: Tool::YtDlp, found: Some(("/usr/bin/yt-dlp".into(), None)), outdated: false, fix: None };
        assert!(found.ok());
        assert!(!Check { outdated: true, ..found.clone() }.ok());
        assert!(!Check { found: None, ..found }.ok());
    }
}
