//! Renders `assets/logo.svg` (and `logo-small.svg` for 16-24 px) into every icon file:
//! - `assets/icon.ico`        exe / installer / tray icon (16..256 px, PNG-compressed entries)
//! - `assets/logo-64.rgba`    raw RGBA for the window/taskbar icon (embedded at compile time)
//! - `assets/logo-128.rgba`   raw RGBA for the logo drawn in the app header
//! - `assets/logo-256.png`    for the README
//!
//! Run after editing the SVG:  cargo run --example make_icons
use resvg::{tiny_skia, usvg};

fn render(tree: &usvg::Tree, size: u32) -> tiny_skia::Pixmap {
    let mut pm = tiny_skia::Pixmap::new(size, size).expect("pixmap");
    let s = size as f32 / tree.size().width();
    resvg::render(tree, tiny_skia::Transform::from_scale(s, s), &mut pm.as_mut());
    pm
}

/// tiny-skia stores premultiplied RGBA; icons and egui want straight alpha.
fn unpremultiplied(pm: &tiny_skia::Pixmap) -> Vec<u8> {
    pm.pixels()
        .iter()
        .flat_map(|p| {
            let c = p.demultiply();
            [c.red(), c.green(), c.blue(), c.alpha()]
        })
        .collect()
}

/// BITMAPINFOHEADER + bottom-up BGRA pixels + an all-zero AND mask (alpha does the work).
fn dib_entry(rgba: &[u8], size: u32) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&40u32.to_le_bytes()); // biSize
    out.extend_from_slice(&(size as i32).to_le_bytes()); // width
    out.extend_from_slice(&((size * 2) as i32).to_le_bytes()); // height: XOR + AND masks
    out.extend_from_slice(&1u16.to_le_bytes()); // planes
    out.extend_from_slice(&32u16.to_le_bytes()); // bpp
    out.extend_from_slice(&[0u8; 24]); // compression .. clr_important
    for y in (0..size).rev() {
        for x in 0..size {
            let i = ((y * size + x) * 4) as usize;
            out.extend_from_slice(&[rgba[i + 2], rgba[i + 1], rgba[i], rgba[i + 3]]);
        }
    }
    let mask_row = ((size + 31) / 32 * 4) as usize;
    out.extend(std::iter::repeat(0u8).take(mask_row * size as usize));
    out
}

fn main() {
    let load = |path: &str| {
        let svg = std::fs::read(path).unwrap_or_else(|e| panic!("read {path}: {e}"));
        usvg::Tree::from_data(&svg, &usvg::Options::default()).expect("parse svg")
    };
    let tree = load("assets/logo.svg");
    // A simplified drawing reads better at tray/taskbar sizes.
    let small = load("assets/logo-small.svg");

    // ICO: classic 32-bit bitmap entries for small sizes (every Windows component reads
    // them), PNG only for 256 px where bitmaps would be huge.
    let sizes = [16u32, 20, 24, 32, 40, 48, 64, 128, 256];
    let pngs: Vec<Vec<u8>> = sizes
        .iter()
        .map(|&s| {
            let pm = render(if s <= 24 { &small } else { &tree }, s);
            if s >= 256 {
                pm.encode_png().expect("png")
            } else {
                dib_entry(&unpremultiplied(&pm), s)
            }
        })
        .collect();
    let mut ico = Vec::new();
    ico.extend_from_slice(&[0, 0, 1, 0]);
    ico.extend_from_slice(&(sizes.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * sizes.len() as u32;
    for (s, png) in sizes.iter().zip(&pngs) {
        let b = if *s >= 256 { 0 } else { *s as u8 };
        ico.extend_from_slice(&[b, b, 0, 0]);
        ico.extend_from_slice(&1u16.to_le_bytes()); // planes
        ico.extend_from_slice(&32u16.to_le_bytes()); // bpp
        ico.extend_from_slice(&(png.len() as u32).to_le_bytes());
        ico.extend_from_slice(&offset.to_le_bytes());
        offset += png.len() as u32;
    }
    for png in &pngs {
        ico.extend_from_slice(png);
    }
    std::fs::write("assets/icon.ico", &ico).expect("write ico");

    std::fs::write("assets/logo-64.rgba", unpremultiplied(&render(&tree, 64))).expect("write 64");
    std::fs::write("assets/logo-128.rgba", unpremultiplied(&render(&tree, 128))).expect("write 128");
    std::fs::write("assets/logo-256.png", render(&tree, 256).encode_png().expect("png")).expect("write 256");
    println!("icon.ico {} bytes, sizes {:?}", ico.len(), sizes);
}
