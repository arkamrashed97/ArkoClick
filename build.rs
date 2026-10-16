//! Embed the icon + manifest (DPI awareness, asInvoker) into the exe on Windows.
//! `embed-resource` uses `windres` on the GNU toolchain and `rc.exe`/`llvm-rc` on MSVC.
//! If no resource compiler is available the build still succeeds; the exe just gets the
//! default icon and no manifest.
fn main() {
    println!("cargo:rerun-if-changed=assets/app.rc");
    println!("cargo:rerun-if-changed=assets/icon.ico");
    println!("cargo:rerun-if-changed=assets/app.manifest");
    if let Err(e) = embed_resource::compile("assets/app.rc", embed_resource::NONE).manifest_optional() {
        println!("cargo:warning=resources not embedded (no resource compiler found): {e:?}");
    }
}
