//! Is there a newer release? Asks GitHub at most once a day and, when there
//! is one, says how to get it on this machine: the package that fits how
//! tubetui was installed here, or `cargo install` when none does.

use std::path::Path;
use std::time::{Duration, SystemTime};

use serde_json::Value;

const LATEST: &str = "https://api.github.com/repos/polblancoo/tubetui/releases/latest";
const DOWNLOAD: &str = "https://github.com/polblancoo/tubetui/releases/download";
/// GitHub is asked again only after this long; meanwhile the answer is cached.
const RECHECK: Duration = Duration::from_secs(24 * 3600);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Update {
    /// `0.5.2`.
    pub version: String,
    pub how: Install,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Install {
    /// A command to paste in a terminal.
    Command(String),
    /// A file to download and open (the Windows installer).
    Link(String),
}

/// The newer release for this machine, if any. `cache` keeps the last
/// answer (`<unix time> <tag> <asset> <asset>…`) so GitHub is asked once a day.
pub fn check(cache: &Path) -> Result<Option<Update>, String> {
    let cached = std::fs::read_to_string(cache).ok().and_then(|text| parse_cache(&text));
    let fresh = cached.as_ref().is_some_and(|(at, ..)| now().saturating_sub(*at) < RECHECK.as_secs());
    let (tag, assets) = match cached {
        Some((_, tag, assets)) if fresh => (tag, assets),
        _ => {
            let (tag, assets) = fetch_latest()?;
            let line = format!("{} {tag} {}", now(), assets.join(" "));
            if let Some(dir) = cache.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let _ = std::fs::write(cache, line);
            (tag, assets)
        }
    };
    let exe = std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_default();
    Ok(update_for(env!("CARGO_PKG_VERSION"), &tag, &assets, &exe, std::env::consts::OS, std::env::consts::ARCH))
}

fn fetch_latest() -> Result<(String, Vec<String>), String> {
    let agent: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(10))).build().into();
    let reply = agent
        .get(LATEST)
        .header("User-Agent", concat!("tubetui/", env!("CARGO_PKG_VERSION")))
        .header("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| e.to_string())?;
    let text = reply.into_body().read_to_string().map_err(|e| e.to_string())?;
    let json: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    parse_latest(&json).ok_or_else(|| "respuesta de GitHub sin versión".into())
}

/// Tag and asset names of GitHub's "latest release" answer.
pub fn parse_latest(json: &Value) -> Option<(String, Vec<String>)> {
    let tag = json["tag_name"].as_str()?.to_owned();
    let assets = json["assets"].as_array()?.iter().filter_map(|a| a["name"].as_str().map(str::to_owned)).collect();
    Some((tag, assets))
}

fn parse_cache(text: &str) -> Option<(u64, String, Vec<String>)> {
    let mut words = text.split_whitespace();
    let at = words.next()?.parse().ok()?;
    let tag = words.next()?.to_owned();
    Some((at, tag, words.map(str::to_owned).collect()))
}

fn now() -> u64 {
    SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// `1.2.3` → `(1, 2, 3)`; a leading `v` and anything after `-` are ignored.
fn version(text: &str) -> Option<(u64, u64, u64)> {
    let core = text.trim_start_matches('v').split(['-', '+']).next()?;
    let mut parts = core.split('.').map(|p| p.parse().ok());
    Some((parts.next()??, parts.next().flatten().unwrap_or(0), parts.next().flatten().unwrap_or(0)))
}

/// The update for a machine (`os`, `arch`) running `current` from `exe`,
/// when `tag` is newer.
pub fn update_for(current: &str, tag: &str, assets: &[String], exe: &str, os: &str, arch: &str) -> Option<Update> {
    if version(tag)? <= version(current)? {
        return None;
    }
    let has = |name: &str| assets.iter().any(|a| a == name);
    let url = |name: &str| format!("{DOWNLOAD}/{tag}/{name}");
    let cargo = Install::Command("cargo install tubetui".into());
    let from_cargo = exe.contains(".cargo/bin") || exe.contains(r".cargo\bin");
    let how = match (os, arch) {
        _ if from_cargo => cargo,
        ("linux", "x86_64") if exe.starts_with("/usr/bin/") && has("tubetui_amd64.deb") => Install::Command(format!(
            "curl -LO {} && sudo apt install ./tubetui_amd64.deb",
            url("tubetui_amd64.deb")
        )),
        ("linux", "x86_64") if has("tubetui-x86_64-linux.tar.gz") => Install::Command(format!(
            "curl -L {} | tar xz && install -m755 tubetui/tubetui {exe}",
            url("tubetui-x86_64-linux.tar.gz")
        )),
        ("windows", "x86_64") if has("tubetui-x86_64-windows-setup.exe") => {
            Install::Link(url("tubetui-x86_64-windows-setup.exe"))
        }
        ("macos", "aarch64") if has("tubetui-aarch64-macos.tar.gz") => Install::Command(format!(
            "curl -L {} | tar xz && install -m755 tubetui/tubetui {exe}",
            url("tubetui-aarch64-macos.tar.gz")
        )),
        _ => cargo,
    };
    Some(Update { version: tag.trim_start_matches('v').to_owned(), how })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assets() -> Vec<String> {
        [
            "tubetui_amd64.deb",
            "tubetui-x86_64-linux.tar.gz",
            "tubetui-x86_64-windows-setup.exe",
            "tubetui-x86_64-windows.zip",
            "tubetui-aarch64-macos.tar.gz",
        ]
        .map(String::from)
        .to_vec()
    }

    fn how(exe: &str, os: &str, arch: &str) -> Install {
        update_for("0.5.1", "v0.6.0", &assets(), exe, os, arch).unwrap().how
    }

    #[test]
    fn only_newer_versions_count() {
        assert_eq!(update_for("0.5.1", "v0.5.1", &assets(), "/usr/bin/tubetui", "linux", "x86_64"), None);
        assert_eq!(update_for("0.5.1", "v0.5.0", &assets(), "/usr/bin/tubetui", "linux", "x86_64"), None);
        assert_eq!(update_for("0.5.1", "garbage", &assets(), "/usr/bin/tubetui", "linux", "x86_64"), None);
        let update = update_for("0.5.1", "v0.10.0", &assets(), "/usr/bin/tubetui", "linux", "x86_64").unwrap();
        assert_eq!(update.version, "0.10.0", "numbers, not text: 10 > 5");
        assert!(version("1.2.3-beta") == Some((1, 2, 3)) && version("v2") == Some((2, 0, 0)));
    }

    #[test]
    fn the_package_matches_how_it_was_installed() {
        let deb = how("/usr/bin/tubetui", "linux", "x86_64");
        assert_eq!(
            deb,
            Install::Command(
                "curl -LO https://github.com/polblancoo/tubetui/releases/download/v0.6.0/tubetui_amd64.deb \
                 && sudo apt install ./tubetui_amd64.deb"
                    .into()
            )
        );
        let Install::Command(tar) = how("/home/u/.local/bin/tubetui", "linux", "x86_64") else { panic!() };
        assert!(tar.contains("tubetui-x86_64-linux.tar.gz | tar xz && install -m755 tubetui/tubetui /home/u/.local/bin/tubetui"));
        assert_eq!(how("/home/u/.cargo/bin/tubetui", "linux", "x86_64"), Install::Command("cargo install tubetui".into()));
        assert_eq!(
            how(r"C:\Users\u\AppData\Local\Programs\tubetui\tubetui.exe", "windows", "x86_64"),
            Install::Link("https://github.com/polblancoo/tubetui/releases/download/v0.6.0/tubetui-x86_64-windows-setup.exe".into())
        );
        assert_eq!(how(r"C:\Users\u\.cargo\bin\tubetui.exe", "windows", "x86_64"), Install::Command("cargo install tubetui".into()));
        let Install::Command(mac) = how("/usr/local/bin/tubetui", "macos", "aarch64") else { panic!() };
        assert!(mac.contains("tubetui-aarch64-macos.tar.gz"));
        // No package for these machines: build it.
        assert_eq!(how("/usr/local/bin/tubetui", "macos", "x86_64"), Install::Command("cargo install tubetui".into()));
        assert_eq!(how("/usr/bin/tubetui", "linux", "aarch64"), Install::Command("cargo install tubetui".into()));
        // A release without the .deb: the tarball.
        let only_tar = vec!["tubetui-x86_64-linux.tar.gz".to_string()];
        let update = update_for("0.5.1", "v0.6.0", &only_tar, "/usr/bin/tubetui", "linux", "x86_64").unwrap();
        assert!(matches!(update.how, Install::Command(c) if c.contains("tar xz")));
    }

    #[test]
    fn reads_github_and_the_cache() {
        let json = serde_json::json!({ "tag_name": "v0.6.0", "assets": [{ "name": "a.deb" }, { "name": "b.zip" }] });
        assert_eq!(parse_latest(&json), Some(("v0.6.0".into(), vec!["a.deb".into(), "b.zip".into()])));
        assert_eq!(parse_latest(&serde_json::json!({ "message": "Not Found" })), None);
        assert_eq!(parse_cache("1700000000 v0.6.0 a.deb b.zip"), Some((1_700_000_000, "v0.6.0".into(), vec!["a.deb".into(), "b.zip".into()])));
        assert_eq!(parse_cache(""), None);
    }
}
