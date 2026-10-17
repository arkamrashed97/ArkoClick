//! Live check of the updater against GitHub: fetch the latest release, compare with the
//! running version, then download and verify the installer (it is not run).
//!     cargo run --example update_probe
//! Set ARKOCLICK_FAKE_VERSION=0.1.0 to pretend an older version is running.
use arkoclick::update;

fn main() {
    let current = update::current_version();
    println!("running version: {}", update::fmt_version(current));
    let rel = match update::fetch_latest() {
        Ok(r) => r,
        Err(e) => {
            println!("check failed: {e}");
            std::process::exit(1);
        }
    };
    println!("latest on GitHub: {} ({})", rel.version_text(), rel.page);
    println!("installer: {:?}  size {}  sha256 {:?}", rel.setup_url, rel.setup_size, rel.setup_sha256);
    if rel.version <= current {
        println!("up to date");
        return;
    }
    println!("update available, downloading…");
    match update::download_setup(&rel) {
        Ok(p) => {
            println!("downloaded and verified: {}", p.display());
            let _ = std::fs::remove_file(p);
        }
        Err(e) => println!("download failed: {e}"),
    }
    println!("installed copy: {}", update::is_installed_copy());
}
