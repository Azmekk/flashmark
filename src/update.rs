//! Self-update from GitHub releases.
//!
//! Windows locks a running executable against writes but allows renaming it,
//! so the swap is: download to `flashmark.exe.new`, rename the running exe to
//! `flashmark.exe.old`, rename `.new` into place. The `.old` backup cannot be
//! deleted while this process lives; the next update run removes it.

use std::fs;
use std::io::{self, Error};

const REPO: &str = "Azmekk/flashmark";
pub const CURRENT: &str = env!("CARGO_PKG_VERSION");

fn get(url: &str) -> io::Result<ureq::http::Response<ureq::Body>> {
    ureq::get(url)
        .header("User-Agent", concat!("flashmark/", env!("CARGO_PKG_VERSION")))
        .call()
        .map_err(|e| Error::other(format!("GET {url}: {e}")))
}

pub fn latest_version() -> io::Result<String> {
    let mut res = get(&format!(
        "https://api.github.com/repos/{REPO}/releases/latest"
    ))?;
    let body = res
        .body_mut()
        .read_to_string()
        .map_err(|e| Error::other(e.to_string()))?;
    let doc: serde_json::Value = serde_json::from_str(&body).map_err(Error::other)?;
    let tag = doc["tag_name"]
        .as_str()
        .ok_or_else(|| Error::other("release response carries no tag_name"))?;
    Ok(tag.trim_start_matches('v').to_string())
}

fn parse_version(v: &str) -> Option<(u64, u64, u64)> {
    let mut it = v.trim().trim_start_matches('v').split('.');
    let major = it.next()?.parse().ok()?;
    let minor = it.next()?.parse().ok()?;
    let patch = it.next()?.parse().ok()?;
    Some((major, minor, patch))
}

pub fn is_newer(candidate: &str, current: &str) -> bool {
    match (parse_version(candidate), parse_version(current)) {
        (Some(c), Some(cur)) => c > cur,
        _ => false,
    }
}

pub enum Outcome {
    UpToDate,
    Updated { to: String },
}

pub fn self_update() -> io::Result<Outcome> {
    let latest = latest_version()?;
    if !is_newer(&latest, CURRENT) {
        return Ok(Outcome::UpToDate);
    }

    let exe = std::env::current_exe()?;
    let new_path = exe.with_extension("exe.new");
    let old_path = exe.with_extension("exe.old");
    let _ = fs::remove_file(&old_path); // stale backup from a previous update

    let mut res = get(&format!(
        "https://github.com/{REPO}/releases/latest/download/flashmark.exe"
    ))?;
    let bytes = res
        .body_mut()
        .read_to_vec()
        .map_err(|e| Error::other(e.to_string()))?;
    if bytes.len() < 200_000 {
        return Err(Error::other(format!(
            "downloaded binary is implausibly small ({} bytes) — aborting",
            bytes.len()
        )));
    }
    fs::write(&new_path, &bytes)?;

    fs::rename(&exe, &old_path)?;
    if let Err(e) = fs::rename(&new_path, &exe) {
        let _ = fs::rename(&old_path, &exe); // roll the running exe back
        return Err(e);
    }
    Ok(Outcome::Updated { to: latest })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_ordering() {
        assert!(is_newer("0.2.0", "0.1.9"));
        assert!(is_newer("1.0.0", "0.9.9"));
        assert!(is_newer("v0.1.1", "0.1.0"));
        assert!(!is_newer("0.1.0", "0.1.0"));
        assert!(!is_newer("0.1.0", "0.1.1"));
        assert!(!is_newer("garbage", "0.1.0"));
    }
}
