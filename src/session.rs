//! The YouTube login: cookies exported from a browser by yt-dlp, and the
//! request signature YouTube's own web client uses.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime};

const ORIGIN: &str = "https://www.youtube.com";
/// Reading cookies from the browser is slow, so the export is reused for a day.
const COOKIES_MAX_AGE: Duration = Duration::from_secs(24 * 3600);

/// Chromium-family browsers: yt-dlp name and config dir relative to `$HOME`.
const CHROMIUM: &[(&str, &str)] = &[
    ("brave", ".config/BraveSoftware/Brave-Browser"),
    ("chrome", ".config/google-chrome"),
    ("chromium", ".config/chromium"),
    ("vivaldi", ".config/vivaldi"),
    ("edge", ".config/microsoft-edge"),
    ("opera", ".config/opera"),
];

/// Browser profiles with cookies under `home`, as yt-dlp
/// `--cookies-from-browser` specs (`brave+KEYRING:Default`, `firefox:abcd.default`).
pub fn scan_browsers(home: &Path, keyring: &str) -> Vec<String> {
    let keyring = if keyring.is_empty() { String::new() } else { format!("+{keyring}") };
    let mut found = Vec::new();
    for (name, dir) in CHROMIUM {
        for profile in subdirs(&home.join(dir)) {
            let is_profile = profile == "Default" || profile.starts_with("Profile ");
            let base = home.join(dir).join(&profile);
            if is_profile && (base.join("Cookies").exists() || base.join("Network/Cookies").exists()) {
                found.push(format!("{name}{keyring}:{profile}"));
            }
        }
    }
    for profile in subdirs(&home.join(".mozilla/firefox")) {
        if home.join(".mozilla/firefox").join(&profile).join("cookies.sqlite").exists() {
            found.push(format!("firefox:{profile}"));
        }
    }
    found
}

fn subdirs(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().is_dir())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// Cookie export for `spec`, one file per browser profile.
pub fn cookies_path(cache_dir: &Path, spec: &str) -> PathBuf {
    let safe: String = spec.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
    cache_dir.join(format!("cookies-{safe}.txt"))
}

/// Exports the browser's cookies to `path` unless a fresh export exists.
pub fn refresh_cookies(spec: &str, path: &Path, force: bool) -> Result<(), String> {
    if !force && is_fresh(path, COOKIES_MAX_AGE) {
        return Ok(());
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    // Any authenticated request makes yt-dlp write the jar; `-I 0` downloads nothing.
    let output = Command::new("yt-dlp")
        .args(["-q", "--skip-download", "--flat-playlist", "-I", "0", "--cookies-from-browser", spec, "--cookies"])
        .arg(path)
        .arg(":ytsubs")
        .output()
        .map_err(|e| format!("no se pudo ejecutar yt-dlp: {e}"))?;
    if path.exists() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!("no se pudo leer la sesión de {spec}: {}", stderr.lines().last().unwrap_or("sin detalle")))
    }
}

pub fn is_fresh(path: &Path, max_age: Duration) -> bool {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .is_ok_and(|t| SystemTime::now().duration_since(t).is_ok_and(|age| age < max_age))
}

/// youtube.com cookies from a Netscape cookie file (`#HttpOnly_` lines included).
pub fn parse_cookies(text: &str) -> HashMap<String, String> {
    text.lines()
        .filter_map(|line| {
            let line = line.strip_prefix("#HttpOnly_").unwrap_or(line);
            if line.starts_with('#') {
                return None;
            }
            let fields: Vec<&str> = line.split('\t').collect();
            match fields.as_slice() {
                [domain, _, _, _, _, name, value] if domain.ends_with("youtube.com") => {
                    Some((name.to_string(), value.to_string()))
                }
                _ => None,
            }
        })
        .collect()
}

/// `Authorization` header for YouTube's internal API, or `None` when the
/// cookies carry no session.
pub fn authorization(cookies: &HashMap<String, String>, timestamp: u64) -> Option<String> {
    let third_party = cookies.get("__Secure-3PAPISID");
    let sapisid = cookies.get("SAPISID").or(third_party)?;
    let mut header = format!("SAPISIDHASH {}", sapisidhash(timestamp, sapisid));
    if let Some(value) = third_party {
        header.push_str(&format!(" SAPISID3PHASH {}", sapisidhash(timestamp, value)));
    }
    Some(header)
}

fn sapisidhash(timestamp: u64, sapisid: &str) -> String {
    let digest = sha1_smol::Sha1::from(format!("{timestamp} {sapisid} {ORIGIN}")).digest();
    format!("{timestamp}_{digest}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::temp_dir;

    #[test]
    fn sapisidhash_matches_youtube_web_client() {
        // Reference computed with Python's hashlib, as in ytv.
        assert_eq!(sapisidhash(1_700_000_000, "abcSAPISID"), "1700000000_b4d59c573190b5106be035dd4b3e304f7edd9c46");
    }

    #[test]
    fn authorization_needs_a_sapisid() {
        assert_eq!(authorization(&HashMap::new(), 1), None);
        let only_3p = HashMap::from([("__Secure-3PAPISID".to_string(), "abcSAPISID".to_string())]);
        let header = authorization(&only_3p, 1_700_000_000).unwrap();
        assert_eq!(
            header,
            "SAPISIDHASH 1700000000_b4d59c573190b5106be035dd4b3e304f7edd9c46 \
             SAPISID3PHASH 1700000000_b4d59c573190b5106be035dd4b3e304f7edd9c46"
        );
    }

    #[test]
    fn parses_youtube_cookies_only() {
        let text = "# Netscape HTTP Cookie File\n\
            .youtube.com\tTRUE\t/\tTRUE\t0\tSAPISID\tabc\n\
            #HttpOnly_.youtube.com\tTRUE\t/\tTRUE\t0\tSID\tsecret\n\
            .google.com\tTRUE\t/\tTRUE\t0\tNID\tnope\n\
            broken line\n";
        let cookies = parse_cookies(text);
        assert_eq!(cookies.len(), 2);
        assert_eq!(cookies["SAPISID"], "abc");
        assert_eq!(cookies["SID"], "secret");
    }

    #[test]
    fn scan_finds_chromium_and_firefox_profiles() {
        let home = temp_dir("scan");
        let touch = |p: &str| {
            let path = home.join(p);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, "").unwrap();
        };
        touch(".config/BraveSoftware/Brave-Browser/Default/Cookies");
        touch(".config/BraveSoftware/Brave-Browser/Profile 1/Network/Cookies");
        touch(".config/BraveSoftware/Brave-Browser/System Profile/Cookies");
        touch(".config/google-chrome/Default/Preferences");
        touch(".mozilla/firefox/ab12.default-release/cookies.sqlite");
        assert_eq!(
            scan_browsers(&home, "GNOMEKEYRING"),
            ["brave+GNOMEKEYRING:Default", "brave+GNOMEKEYRING:Profile 1", "firefox:ab12.default-release"]
        );
        assert_eq!(scan_browsers(&home, "")[0], "brave:Default");
    }

    #[test]
    fn cookies_path_is_per_profile_and_filename_safe() {
        let p = cookies_path(Path::new("/c"), "brave+GNOMEKEYRING:Profile 1");
        assert_eq!(p, Path::new("/c/cookies-brave_GNOMEKEYRING_Profile_1.txt"));
    }

    #[test]
    fn freshness() {
        let path = temp_dir("fresh").join("f");
        assert!(!is_fresh(&path, Duration::from_secs(60)));
        std::fs::write(&path, "").unwrap();
        assert!(is_fresh(&path, Duration::from_secs(60)));
        assert!(!is_fresh(&path, Duration::ZERO));
    }
}
