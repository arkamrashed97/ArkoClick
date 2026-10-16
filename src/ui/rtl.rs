//! Arabic display support for egui, which draws glyphs left-to-right and does no shaping.
//!
//! `visual()` turns a logical string into what should be drawn:
//! 1. letters are replaced by their joined forms (Arabic Presentation Forms, via
//!    `ar-reshaper`), so they connect as in normal Arabic writing;
//! 2. the Unicode Bidirectional Algorithm (`unicode-bidi`) reorders the line into visual
//!    order, keeping numbers and English words left-to-right and mirroring brackets.
//!
//! Results are cached, so labels cost nothing after their first frame.

use std::cell::RefCell;
use std::collections::HashMap;
use unicode_bidi::{BidiInfo, Level};

thread_local! {
    static CACHE: RefCell<HashMap<(String, bool), String>> = RefCell::new(HashMap::new());
}

pub fn has_rtl(s: &str) -> bool {
    s.chars().any(|c| matches!(c as u32, 0x0590..=0x08FF | 0xFB1D..=0xFDFF | 0xFE70..=0xFEFF))
}

fn mirror(c: char) -> char {
    match c {
        '(' => ')',
        ')' => '(',
        '[' => ']',
        ']' => '[',
        '{' => '}',
        '}' => '{',
        '<' => '>',
        '>' => '<',
        '«' => '»',
        '»' => '«',
        _ => c,
    }
}

/// Visual-order, joined form of one line of text. `rtl_base` sets the paragraph direction
/// (true for the Arabic UI, where e.g. a leading number belongs on the right).
pub fn visual(text: &str, rtl_base: bool) -> String {
    if !has_rtl(text) && !rtl_base {
        return text.to_string();
    }
    if !has_rtl(text) && text.chars().all(|c| c.is_ascii()) && !text.contains(['(', ')', '[', ']']) {
        // Pure Latin text in an RTL paragraph still reads left-to-right.
        return text.to_string();
    }
    let key = (text.to_string(), rtl_base);
    if let Some(hit) = CACHE.with(|c| c.borrow().get(&key).cloned()) {
        return hit;
    }
    let out = text.split('\n').map(|line| visual_line(line, rtl_base)).collect::<Vec<_>>().join("\n");
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        if c.len() > 4000 {
            c.clear();
        }
        c.insert(key, out.clone());
    });
    out
}

fn visual_line(line: &str, rtl_base: bool) -> String {
    let shaped = ar_reshaper::reshape_line(line);
    let base = if rtl_base { Some(Level::rtl()) } else { None };
    let info = BidiInfo::new(&shaped, base);
    let Some(para) = info.paragraphs.first() else { return shaped };
    let (levels, runs) = info.visual_runs(para, para.range.clone());
    let mut out = String::with_capacity(shaped.len());
    for run in runs {
        let seg = &shaped[run.clone()];
        if levels[run.start].is_rtl() {
            out.extend(seg.chars().rev().map(mirror));
        } else {
            out.push_str(seg);
        }
    }
    out
}

/// Break a logical paragraph into lines that fit `max_width` (measured on the shaped
/// text), then return each line in visual form. Needed because egui would otherwise wrap
/// the already-reversed string and put the end of the sentence on the first line.
pub fn wrap(text: &str, rtl_base: bool, max_width: f32, measure: impl Fn(&str) -> f32) -> Vec<String> {
    let mut lines = Vec::new();
    for para in text.split('\n') {
        let mut cur = String::new();
        for word in para.split(' ') {
            let candidate = if cur.is_empty() { word.to_string() } else { format!("{cur} {word}") };
            if !cur.is_empty() && measure(&visual(&candidate, rtl_base)) > max_width {
                lines.push(visual(&cur, rtl_base));
                cur = word.to_string();
            } else {
                cur = candidate;
            }
        }
        lines.push(visual(&cur, rtl_base));
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn joins_and_reverses_arabic() {
        // "سلام" -> joined presentation forms, drawn right-to-left (so reversed in memory).
        let v = visual("سلام", true);
        assert!(v.chars().all(|c| (0xFE70..=0xFEFF).contains(&(c as u32))), "{v:?}");
        assert_eq!(v.chars().count(), 3, "lam-alef ligature: {v:?}");
        // Last logical letter (meem) comes first visually. Alef never joins forward, so the
        // meem after it takes its isolated form.
        assert_eq!(v.chars().next(), Some('\u{FEE1}'));
    }

    #[test]
    fn numbers_and_latin_keep_their_order() {
        let v = visual("كل 250 ms", true);
        assert!(v.contains("250") && v.contains("ms"), "{v:?}");
        // In an RTL paragraph the Latin run ends up on the left.
        assert!(v.starts_with("ms"), "{v:?}");
    }

    #[test]
    fn plain_english_untouched() {
        assert_eq!(visual("Start all", false), "Start all");
        assert_eq!(visual("Start all", true), "Start all");
    }
}
