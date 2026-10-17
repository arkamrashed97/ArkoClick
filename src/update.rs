//! Update checker: asks GitHub for the latest release, compares versions, downloads the
//! installer, verifies it against the SHA-256 checksum GitHub publishes for every release
//! file, and launches it.
//!
//! Network access goes through Windows' own `curl.exe` (System32, Windows 10 1803+), so the
//! app ships no TLS stack of its own. Only files from this repository's release downloads
//! are accepted.

use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;

pub const REPO: &str = "arkamrashed97/ArkoClick";
pub const SETUP_ASSET: &str = "ArkoClick-Setup.exe";
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

pub type Version = (u32, u32, u32);

#[derive(Debug, Clone, PartialEq)]
pub struct Release {
    pub version: Version,
    pub tag: String,
    /// Release page on GitHub ("What's new").
    pub page: String,
    pub notes: String,
    pub setup_url: Option<String>,
    /// Lower-case hex SHA-256 of the installer, from GitHub's asset digest.
    pub setup_sha256: Option<String>,
    pub setup_size: u64,
}

impl Release {
    pub fn version_text(&self) -> String {
        fmt_version(self.version)
    }
}

pub fn fmt_version(v: Version) -> String {
    format!("{}.{}.{}", v.0, v.1, v.2)
}

/// "v1.2.3", "1.2.3", "1.2" -> (1, 2, 3) / (1, 2, 0). Anything after the numbers is ignored.
pub fn parse_version(s: &str) -> Option<Version> {
    let s = s.trim().trim_start_matches(['v', 'V']);
    let core = s.split(|c: char| c == '-' || c == '+' || c.is_whitespace()).next()?;
    let mut parts = core.split('.').map(|p| p.parse::<u32>());
    let major = parts.next()?.ok()?;
    let minor = parts.next().unwrap_or(Ok(0)).ok()?;
    let patch = parts.next().unwrap_or(Ok(0)).ok()?;
    Some((major, minor, patch))
}

/// The running version. `ARKOCLICK_FAKE_VERSION` overrides it, to exercise the update path
/// against a real release during testing.
pub fn current_version() -> Version {
    std::env::var("ARKOCLICK_FAKE_VERSION")
        .ok()
        .and_then(|v| parse_version(&v))
        .or_else(|| parse_version(env!("CARGO_PKG_VERSION")))
        .unwrap_or((0, 0, 0))
}

#[derive(Deserialize)]
struct ApiAsset {
    name: String,
    browser_download_url: String,
    #[serde(default)]
    size: u64,
    #[serde(default)]
    digest: Option<String>,
}

#[derive(Deserialize)]
struct ApiRelease {
    tag_name: String,
    html_url: String,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    assets: Vec<ApiAsset>,
}

/// Parse GitHub's `releases/latest` response. Downloads must come from this repository.
pub fn parse_release(json: &str) -> Result<Release, String> {
    let r: ApiRelease = serde_json::from_str(json).map_err(|e| format!("unexpected reply from GitHub: {e}"))?;
    if r.draft || r.prerelease {
        return Err("latest release is not a final release".into());
    }
    let version = parse_version(&r.tag_name).ok_or_else(|| format!("unreadable version tag {:?}", r.tag_name))?;
    let allowed = format!("https://github.com/{REPO}/releases/download/");
    let setup = r.assets.iter().find(|a| a.name.eq_ignore_ascii_case(SETUP_ASSET));
    let (setup_url, setup_sha256, setup_size) = match setup {
        Some(a) if a.browser_download_url.starts_with(&allowed) => (
            Some(a.browser_download_url.clone()),
            a.digest.as_deref().and_then(|d| d.strip_prefix("sha256:")).map(|h| h.to_ascii_lowercase()),
            a.size,
        ),
        Some(a) => return Err(format!("refusing download from unexpected address {}", a.browser_download_url)),
        None => (None, None, 0),
    };
    let page = if r.html_url.starts_with(&format!("https://github.com/{REPO}/")) {
        r.html_url
    } else {
        format!("https://github.com/{REPO}/releases/latest")
    };
    Ok(Release { version, tag: r.tag_name, page, notes: r.body.unwrap_or_default(), setup_url, setup_sha256, setup_size })
}

fn curl() -> PathBuf {
    let root = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".into());
    Path::new(&root).join("System32").join("curl.exe")
}

/// Ask GitHub for the latest release.
pub fn fetch_latest() -> Result<Release, String> {
    let url = format!("https://api.github.com/repos/{REPO}/releases/latest");
    let out = Command::new(curl())
        .args(["-sS", "-L", "--fail", "--max-time", "20"])
        .args(["-H", "Accept: application/vnd.github+json"])
        .args(["-H", &format!("User-Agent: ArkoClick/{}", env!("CARGO_PKG_VERSION"))])
        .arg(&url)
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| format!("could not run curl: {e}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(if err.is_empty() { "no answer from GitHub".into() } else { err });
    }
    parse_release(&String::from_utf8_lossy(&out.stdout))
}

pub fn sha256_hex(path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    Ok(hex(&Sha256::digest(&bytes)))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Download the release's installer into the temp folder and verify it. Returns its path.
pub fn download_setup(rel: &Release) -> Result<PathBuf, String> {
    let url = rel.setup_url.as_deref().ok_or("this release has no installer file")?;
    let dest = std::env::temp_dir().join(format!("ArkoClick-Setup-{}.exe", rel.version_text()));
    let _ = std::fs::remove_file(&dest);
    let out = Command::new(curl())
        .args(["-sS", "-L", "--fail", "--max-time", "600", "-o"])
        .arg(&dest)
        .arg(url)
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| format!("could not run curl: {e}"))?;
    if !out.status.success() {
        let _ = std::fs::remove_file(&dest);
        return Err(format!("download failed: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    verify(&dest, rel).map_err(|e| {
        let _ = std::fs::remove_file(&dest);
        e
    })?;
    Ok(dest)
}

/// Size and checksum must match what GitHub reports for the file.
pub fn verify(path: &Path, rel: &Release) -> Result<(), String> {
    let size = std::fs::metadata(path).map_err(|e| e.to_string())?.len();
    if rel.setup_size > 0 && size != rel.setup_size {
        return Err(format!("downloaded file has the wrong size ({size} instead of {})", rel.setup_size));
    }
    if let Some(expected) = &rel.setup_sha256 {
        let actual = sha256_hex(path)?;
        if &actual != expected {
            return Err("downloaded file failed the checksum test".into());
        }
    }
    Ok(())
}

/// True when this exe is the installed copy (per-user install folder), so updating through
/// the installer replaces it. Portable copies get the download page instead.
pub fn is_installed_copy() -> bool {
    let (Ok(exe), Some(local)) = (std::env::current_exe(), dirs::data_local_dir()) else { return false };
    let installed = local.join("Programs").join(crate::config::APP_TITLE);
    exe.parent().map_or(false, |dir| dir.eq(&installed) || dir.starts_with(&installed))
}

/// Run the installer quietly (progress window only). `/UPDATE=1` makes it wait for this app
/// to exit and start the new version when done.
pub fn launch_installer(path: &Path, arabic: bool) -> Result<(), String> {
    Command::new(path)
        .args(["/SILENT", "/SUPPRESSMSGBOXES", "/NORESTART", "/UPDATE=1"])
        .arg(if arabic { "/LANG=arabic" } else { "/LANG=english" })
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("could not start the installer: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions() {
        assert_eq!(parse_version("v0.3.0"), Some((0, 3, 0)));
        assert_eq!(parse_version("1.2"), Some((1, 2, 0)));
        assert_eq!(parse_version("V2.10.4-beta"), Some((2, 10, 4)));
        assert_eq!(parse_version("latest"), None);
        assert!((0, 10, 0) > (0, 9, 9), "numeric, not text, comparison");
    }

    fn sample(url: &str) -> String {
        format!(
            r#"{{"tag_name":"v0.4.0","html_url":"https://github.com/{REPO}/releases/tag/v0.4.0","body":"notes",
            "draft":false,"prerelease":false,"assets":[
              {{"name":"ArkoClick-portable.zip","browser_download_url":"https://github.com/{REPO}/releases/download/v0.4.0/ArkoClick-portable.zip","size":10,"digest":"sha256:aa"}},
              {{"name":"ArkoClick-Setup.exe","browser_download_url":"{url}","size":123,"digest":"sha256:ABCDEF"}}]}}"#
        )
    }

    #[test]
    fn parses_release_and_digest() {
        let url = format!("https://github.com/{REPO}/releases/download/v0.4.0/ArkoClick-Setup.exe");
        let r = parse_release(&sample(&url)).unwrap();
        assert_eq!(r.version, (0, 4, 0));
        assert_eq!(r.setup_url.as_deref(), Some(url.as_str()));
        assert_eq!(r.setup_sha256.as_deref(), Some("abcdef"));
        assert_eq!(r.setup_size, 123);
    }

    #[test]
    fn rejects_foreign_download_address() {
        let err = parse_release(&sample("https://example.com/evil.exe")).unwrap_err();
        assert!(err.contains("unexpected address"), "{err}");
    }

    #[test]
    fn checksum_and_size_are_enforced() {
        let p = std::env::temp_dir().join("arkoclick-verify-test.bin");
        std::fs::write(&p, b"abc").unwrap();
        // Well-known SHA-256 of "abc".
        let good = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        assert_eq!(sha256_hex(&p).unwrap(), good);
        let mut rel = Release {
            version: (9, 9, 9),
            tag: "v9.9.9".into(),
            page: String::new(),
            notes: String::new(),
            setup_url: None,
            setup_sha256: Some(good.into()),
            setup_size: 3,
        };
        assert!(verify(&p, &rel).is_ok());
        rel.setup_sha256 = Some("00".repeat(32));
        assert!(verify(&p, &rel).is_err(), "wrong checksum must fail");
        rel.setup_sha256 = Some(good.into());
        rel.setup_size = 4;
        assert!(verify(&p, &rel).is_err(), "wrong size must fail");
        let _ = std::fs::remove_file(&p);
    }
}
