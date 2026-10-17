//! Persistent configuration: profiles of clickers + global settings.
//! Stored as JSON in `%APPDATA%\ArkoClick\config.json`. Older single-list configs
//! (v0.1) are migrated on load.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const APP_TITLE: &str = "ArkoClick";
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const DEVELOPER: &str = "Arkam Rashed";
pub const DEVELOPER_EMAIL: &str = "arko313m@gmail.com";
pub const DEVELOPER_PHONE: &str = "+201008446123";

// ---------------------------------------------------------------------------- basic enums

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum MouseButton {
    #[default]
    Left,
    Right,
    Middle,
}

impl MouseButton {
    pub fn label(self) -> &'static str {
        match self {
            MouseButton::Left => "Left",
            MouseButton::Right => "Right",
            MouseButton::Middle => "Middle",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ClickKind {
    #[default]
    Single,
    Double,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ClickMode {
    /// PostMessage straight to the target window. Cursor never moves; works while the
    /// window is covered, minimized or on another virtual desktop.
    #[default]
    Background,
    /// SendInput at the point's current screen position. Moves the real cursor; works
    /// with games that read raw input, but the window must be visible.
    Foreground,
}

// ---------------------------------------------------------------------------- target

/// How we identify the target window so we can re-bind to it after it is
/// closed and reopened (the HWND itself is only valid for the window's lifetime).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct TargetInfo {
    pub title: String,
    pub class: String,
    pub process: String,
    /// Runtime-only handle. Not persisted: HWNDs are recycled between sessions.
    #[serde(skip)]
    pub hwnd: Option<isize>,
}

impl TargetInfo {
    pub fn is_set(&self) -> bool {
        !self.class.is_empty() || !self.process.is_empty() || self.hwnd.is_some()
    }
    /// Chromium-based apps (Chrome, Edge, Electron, Brave...) only register clicks that are
    /// posted to the top-level `Chrome_WidgetWin_*` window, not to the render child.
    pub fn is_chromium(&self) -> bool {
        self.class.starts_with("Chrome_WidgetWin")
    }
}

/// Browsers whose window titles carry the page title ("Page - Google Chrome").
pub const BROWSERS: &[&str] = &["chrome.exe", "msedge.exe", "brave.exe", "firefox.exe", "opera.exe", "vivaldi.exe"];

impl TargetInfo {
    pub fn is_browser(&self) -> bool {
        BROWSERS.iter().any(|b| self.process.eq_ignore_ascii_case(b))
    }
}

/// Suggested "title contains" text for linking a task to a window: the site name for a browser
/// page ("YouTube" from "Watch later - YouTube - Google Chrome"), empty (any window of the
/// program) otherwise. The user can edit it.
pub fn suggested_title_hint(t: &TargetInfo) -> String {
    if !t.is_browser() {
        return String::new();
    }
    let norm = t.title.replace(" \u{2014} ", " - ").replace(" \u{2013} ", " - ");
    let mut parts: Vec<&str> = norm.split(" - ").map(str::trim).filter(|s| !s.is_empty()).collect();
    // Drop the browser name ("Google Chrome", "Microsoft Edge", "Mozilla Firefox"...).
    if parts.len() > 1 {
        parts.pop();
    }
    parts.last().map(|s| s.to_string()).unwrap_or_default()
}

/// "Ask to start this task when its program or page opens."
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Trigger {
    pub enabled: bool,
    /// Program file name ("chrome.exe"). Empty = the task's own target program.
    pub process: String,
    /// Only windows whose title contains this text (any case). Empty = any window.
    pub title_contains: String,
}

/// Where inside the target's client area to click.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ClickPoint {
    /// Client-area pixel coordinates captured at pick time.
    pub x: i32,
    pub y: i32,
    /// Client-area size at pick time. Used to scale when `relative` is on and as a
    /// fallback while the window is minimized (minimized windows report a 0x0 client rect).
    pub client_w: i32,
    pub client_h: i32,
    /// Scale the point with the window's current client size (survives resize/maximize).
    pub relative: bool,
}

impl Default for ClickPoint {
    fn default() -> Self {
        Self { x: 0, y: 0, client_w: 0, client_h: 0, relative: true }
    }
}

impl ClickPoint {
    /// Resolve the client coordinates for the given current client size.
    pub fn resolve(&self, cur_w: i32, cur_h: i32) -> (i32, i32) {
        if self.relative && self.client_w > 0 && self.client_h > 0 && cur_w > 0 && cur_h > 0 {
            let rx = self.x as f64 / self.client_w as f64;
            let ry = self.y as f64 / self.client_h as f64;
            ((rx * cur_w as f64).round() as i32, (ry * cur_h as f64).round() as i32)
        } else {
            (self.x, self.y)
        }
    }
}

// ---------------------------------------------------------------------------- steps

/// One thing a clicker does.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Action {
    Click { point: ClickPoint, button: MouseButton, kind: ClickKind },
    /// Press and release one key (virtual-key code), delivered to the window's focused control.
    Key { vk: u32 },
    /// Type a string (any Unicode, Arabic included) as WM_CHAR messages.
    Text { text: String },
}

impl Default for Action {
    fn default() -> Self {
        Action::Click { point: ClickPoint::default(), button: MouseButton::Left, kind: ClickKind::Single }
    }
}

impl Action {
    pub fn kind_label(&self) -> &'static str {
        match self {
            Action::Click { .. } => "Click",
            Action::Key { .. } => "Key",
            Action::Text { .. } => "Text",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Step {
    pub action: Action,
    /// Wait after this step before running the next one (ms).
    pub wait_ms: u64,
    /// Zoomed screenshot of the picked spot (THUMB_SIZE² RGBA, base64), shown in the UI so
    /// non-technical users can see exactly what will be clicked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thumb: Option<String>,
    /// Switched-off steps are skipped.
    #[serde(default = "yes")]
    pub enabled: bool,
}

fn yes() -> bool {
    true
}

/// Side length in pixels of the captured area around a picked point.
pub const THUMB_SIZE: u32 = 48;

impl Step {
    pub fn click(point: ClickPoint, wait_ms: u64) -> Self {
        Self::new(Action::Click { point, button: MouseButton::Left, kind: ClickKind::Single }, wait_ms)
    }
    pub fn new(action: Action, wait_ms: u64) -> Self {
        Self { action, wait_ms, thumb: None, enabled: true }
    }
}

// ---------------------------------------------------------------------------- hotkeys

/// A global hotkey. `vk == 0` means "not set". Accepts the old v0.1 format (a bare
/// virtual-key number) when loading.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(from = "HotkeyRepr")]
pub struct Hotkey {
    pub vk: u32,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum HotkeyRepr {
    Vk(u32),
    Full {
        vk: u32,
        #[serde(default)]
        ctrl: bool,
        #[serde(default)]
        alt: bool,
        #[serde(default)]
        shift: bool,
    },
}

impl From<HotkeyRepr> for Hotkey {
    fn from(r: HotkeyRepr) -> Self {
        match r {
            HotkeyRepr::Vk(vk) => Hotkey { vk, ..Default::default() },
            HotkeyRepr::Full { vk, ctrl, alt, shift } => Hotkey { vk, ctrl, alt, shift },
        }
    }
}

impl Hotkey {
    pub const NONE: Hotkey = Hotkey { vk: 0, ctrl: false, alt: false, shift: false };

    pub fn key(vk: u32) -> Self {
        Hotkey { vk, ..Default::default() }
    }
    pub fn is_set(&self) -> bool {
        self.vk != 0
    }
    pub fn has_modifier(&self) -> bool {
        self.ctrl || self.alt || self.shift
    }
    pub fn label(&self) -> String {
        if !self.is_set() {
            return "None".into();
        }
        let mut s = String::new();
        if self.ctrl {
            s.push_str("Ctrl+");
        }
        if self.alt {
            s.push_str("Alt+");
        }
        if self.shift {
            s.push_str("Shift+");
        }
        s.push_str(&vk_label(self.vk));
        s
    }
}

/// Virtual-key codes we know how to name (and offer in key pickers).
pub const KEYS: &[(u32, &str)] = &[
    (0x41, "A"), (0x42, "B"), (0x43, "C"), (0x44, "D"), (0x45, "E"), (0x46, "F"), (0x47, "G"),
    (0x48, "H"), (0x49, "I"), (0x4A, "J"), (0x4B, "K"), (0x4C, "L"), (0x4D, "M"), (0x4E, "N"),
    (0x4F, "O"), (0x50, "P"), (0x51, "Q"), (0x52, "R"), (0x53, "S"), (0x54, "T"), (0x55, "U"),
    (0x56, "V"), (0x57, "W"), (0x58, "X"), (0x59, "Y"), (0x5A, "Z"),
    (0x30, "0"), (0x31, "1"), (0x32, "2"), (0x33, "3"), (0x34, "4"),
    (0x35, "5"), (0x36, "6"), (0x37, "7"), (0x38, "8"), (0x39, "9"),
    (0x70, "F1"), (0x71, "F2"), (0x72, "F3"), (0x73, "F4"), (0x74, "F5"), (0x75, "F6"),
    (0x76, "F7"), (0x77, "F8"), (0x78, "F9"), (0x79, "F10"), (0x7A, "F11"), (0x7B, "F12"),
    (0x20, "Space"), (0x0D, "Enter"), (0x09, "Tab"), (0x1B, "Esc"), (0x08, "Backspace"),
    (0x2D, "Insert"), (0x2E, "Delete"), (0x24, "Home"), (0x23, "End"), (0x21, "PageUp"), (0x22, "PageDown"),
    (0x25, "Left"), (0x26, "Up"), (0x27, "Right"), (0x28, "Down"),
    (0xBD, "-"), (0xBB, "="), (0xBC, ","), (0xBE, "."), (0xBA, ";"), (0xBF, "/"), (0xC0, "`"),
    (0xDB, "["), (0xDC, "\\"), (0xDD, "]"), (0xDE, "'"),
];

pub fn vk_label(vk: u32) -> String {
    KEYS.iter().find(|(k, _)| *k == vk).map(|(_, l)| l.to_string()).unwrap_or_else(|| format!("VK 0x{vk:02X}"))
}

pub fn is_function_key(vk: u32) -> bool {
    (0x70..=0x87).contains(&vk)
}

/// Choose a default hotkey for a new classic clicker that doesn't conflict with existing hotkeys.
pub fn find_available_classic_hotkey(used: &[Hotkey]) -> Hotkey {
    for vk in [0x76 /* F7 */, 0x78 /* F9 */, 0x79 /* F10 */, 0x7A /* F11 */, 0x7B /* F12 */] {
        let candidate = Hotkey::key(vk);
        if !used.contains(&candidate) {
            return candidate;
        }
    }
    Hotkey::NONE
}

// ---------------------------------------------------------------------------- kinds / prefs

/// Which tab a clicker lives in, and therefore which editor it gets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum ClickerKind {
    /// Classic mouse autoclicker: clicks wherever the mouse cursor is located.
    Classic,
    /// One spot, clicked repeatedly.
    #[default]
    Simple,
    /// Any sequence of clicks / keys / text with every option.
    Advanced,
    /// Press a key or type text into a window (optionally clicking a field first).
    Keyboard,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Lang {
    #[default]
    En,
    Ar,
}

impl Lang {
    /// Arabic when the Windows display language is Arabic, English otherwise.
    pub fn system_default() -> Self {
        let id = unsafe { windows::Win32::Globalization::GetUserDefaultUILanguage() };
        if id & 0x3FF == 0x01 {
            Lang::Ar
        } else {
            Lang::En
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ThemePref {
    #[default]
    System,
    Light,
    Dark,
}

// ---------------------------------------------------------------------------- clicker

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ClickerConfig {
    pub id: u64,
    pub name: String,
    /// Which tab the clicker belongs to. `None` only while loading an older config;
    /// resolved by `migrate`.
    pub tab: Option<ClickerKind>,
    pub target: TargetInfo,
    /// Actions run in order, then the sequence repeats.
    pub steps: Vec<Step>,
    pub mode: ClickMode,
    /// Post to the top-level window instead of the deepest child control under the point.
    pub post_to_root: bool,
    /// Random +/- jitter added to each wait (ms). 0 = none.
    pub jitter_ms: u64,
    /// Random +/- jitter added to the click position (px). 0 = none.
    pub jitter_px: i32,
    /// Stop after the sequence ran this many times (for a one-step clicker: this many
    /// clicks). 0 = unlimited.
    #[serde(alias = "max_clicks")]
    pub max_rounds: u64,
    /// Stop after this many seconds. 0 = unlimited.
    pub max_seconds: u64,
    /// Start this clicker automatically when the app launches.
    pub auto_run: bool,
    /// Global hotkey that starts/stops just this clicker.
    pub hotkey: Hotkey,
    /// While running, turn a minimize of the target into "hidden but running" (many apps,
    /// all Chromium browsers included, ignore input while minimized).
    pub hide_when_minimized: bool,
    /// Offer to start this task when its program / browser page comes to the front.
    pub trigger: Trigger,

    // ---- v0.1 fields, read once for migration and never written back.
    #[serde(skip_serializing)]
    point: Option<ClickPoint>,
    #[serde(skip_serializing)]
    button: Option<MouseButton>,
    #[serde(skip_serializing)]
    kind: Option<ClickKind>,
    #[serde(skip_serializing)]
    interval_ms: Option<u64>,
}

impl Default for ClickerConfig {
    fn default() -> Self {
        Self::new(0)
    }
}

pub const DEFAULT_WAIT_MS: u64 = 100;

impl ClickerConfig {
    pub fn new(id: u64) -> Self {
        Self {
            id,
            name: format!("Clicker {id}"),
            tab: Some(ClickerKind::Simple),
            target: TargetInfo::default(),
            steps: vec![Step::click(ClickPoint::default(), DEFAULT_WAIT_MS)],
            mode: ClickMode::Background,
            post_to_root: false,
            jitter_ms: 0,
            jitter_px: 0,
            max_rounds: 0,
            max_seconds: 0,
            auto_run: false,
            hotkey: Hotkey::NONE,
            hide_when_minimized: true,
            trigger: Trigger::default(),
            point: None,
            button: None,
            kind: None,
            interval_ms: None,
        }
    }

    /// Fold v0.1's single point/button/interval into a one-step sequence.
    fn migrate(&mut self) {
        if let Some(point) = self.point.take() {
            let button = self.button.take().unwrap_or_default();
            let kind = self.kind.take().unwrap_or_default();
            let wait_ms = self.interval_ms.take().unwrap_or(DEFAULT_WAIT_MS);
            self.steps = vec![Step::new(Action::Click { point, button, kind }, wait_ms)];
        }
        self.button = None;
        self.kind = None;
        self.interval_ms = None;
        if self.steps.is_empty() {
            self.steps.push(Step::click(ClickPoint::default(), DEFAULT_WAIT_MS));
        }
        if self.tab.is_none() {
            self.tab = Some(self.guess_kind());
        }
    }

    pub fn kind(&self) -> ClickerKind {
        self.tab.unwrap_or_default()
    }

    /// Classify a clicker made before tabs existed.
    fn guess_kind(&self) -> ClickerKind {
        let clicks = self.steps.iter().filter(|s| matches!(s.action, Action::Click { .. })).count();
        let keys = self.steps.len() - clicks;
        match (clicks, keys) {
            (1, 0) => ClickerKind::Simple,
            (0, 1) | (1, 1) if !matches!(self.steps.last().map(|s| &s.action), Some(Action::Click { .. })) => {
                ClickerKind::Keyboard
            }
            _ => ClickerKind::Advanced,
        }
    }

    /// A fresh clicker for the given tab.
    pub fn new_of_kind(id: u64, kind: ClickerKind, n: usize, lang: Lang) -> Self {
        let mut c = Self::new(id);
        c.tab = Some(kind);
        c.name = match (kind, lang) {
            (ClickerKind::Classic, Lang::Ar) => format!("كلاسيكي {n}"),
            (ClickerKind::Classic, Lang::En) => format!("Classic {n}"),
            (ClickerKind::Keyboard, Lang::Ar) => format!("كتابة {n}"),
            (ClickerKind::Keyboard, Lang::En) => format!("Typing {n}"),
            (ClickerKind::Advanced, Lang::Ar) => format!("متقدم {n}"),
            (ClickerKind::Advanced, Lang::En) => format!("Advanced {n}"),
            (ClickerKind::Simple, Lang::Ar) => format!("نقر {n}"),
            (ClickerKind::Simple, Lang::En) => format!("Clicker {n}"),
        };
        if kind == ClickerKind::Keyboard {
            c.steps = vec![Step::new(Action::Key { vk: 0x20 }, 1000)];
        } else if kind == ClickerKind::Classic {
            c.hotkey = Hotkey::key(0x76); // F7 default hotkey
            c.mode = ClickMode::Foreground;
        }
        c
    }

    /// The first click step's point, if any (what "Pick on screen" in the target section sets).
    pub fn first_click_point_mut(&mut self) -> Option<&mut ClickPoint> {
        self.steps.iter_mut().find_map(|s| match &mut s.action {
            Action::Click { point, .. } => Some(point),
            _ => None,
        })
    }

    pub fn has_unpicked_click(&self) -> bool {
        self.steps.iter().any(|s| matches!(&s.action, Action::Click { point, .. } if point.client_w == 0 && point.x == 0 && point.y == 0))
    }

    /// Shortest wait in the sequence; drives the high-resolution timer decision.
    pub fn min_wait_ms(&self) -> u64 {
        self.steps.iter().filter(|s| s.enabled).map(|s| s.wait_ms.max(1)).min().unwrap_or(DEFAULT_WAIT_MS)
    }
}

// ---------------------------------------------------------------------------- profiles

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Profile {
    pub name: String,
    pub clickers: Vec<ClickerConfig>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub profiles: Vec<Profile>,
    pub active_profile: usize,
    pub next_id: u64,
    /// Launch with Windows (HKCU Run key).
    pub autostart: bool,
    /// Start hidden in the tray.
    pub start_minimized: bool,
    /// The window close button hides to the tray instead of quitting.
    pub close_to_tray: bool,
    /// Global hotkey: start/stop every clicker of the active profile.
    pub hotkey_toggle_all: Hotkey,
    /// Global hotkey: emergency stop.
    pub hotkey_stop_all: Hotkey,
    /// v0.1/v0.2 boolean, folded into `theme` on load.
    #[serde(skip_serializing)]
    dark_theme: Option<bool>,
    pub theme: ThemePref,
    /// `None` until first launch picks the system language.
    pub lang: Option<Lang>,
    /// UI text size in points.
    pub font_size: f32,
    /// Look for a new version on GitHub at startup and once a day.
    pub check_updates: bool,
    /// Version the user chose to skip ("v0.4.0"); no reminder for it.
    pub skipped_version: Option<String>,

    /// v0.1 single clicker list, migrated into a "Default" profile on load.
    #[serde(skip_serializing)]
    clickers: Option<Vec<ClickerConfig>>,
}

impl Default for AppConfig {
    fn default() -> Self {
        let lang = Lang::system_default();
        // The starter classic task has no shortcut: a global F7 would be taken from every
        // other program even for people who never use the classic clicker.
        let mut classic = ClickerConfig::new_of_kind(1, ClickerKind::Classic, 1, lang);
        classic.hotkey = Hotkey::NONE;
        let simple = ClickerConfig::new_of_kind(2, ClickerKind::Simple, 1, lang);
        Self {
            profiles: vec![Profile {
                name: if lang == Lang::Ar { "الافتراضي".into() } else { "Default".into() },
                clickers: vec![classic, simple],
            }],
            active_profile: 0,
            next_id: 3,
            autostart: false,
            start_minimized: false,
            close_to_tray: true,
            hotkey_toggle_all: Hotkey::key(0x75), // F6
            hotkey_stop_all: Hotkey::key(0x77),   // F8
            dark_theme: None,
            theme: ThemePref::System,
            lang: Some(lang),
            font_size: 15.0,
            check_updates: true,
            skipped_version: None,
            clickers: None,
        }
    }
}

impl AppConfig {
    pub fn path() -> PathBuf {
        let mut dir = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
        dir.push(APP_TITLE);
        dir.push("config.json");
        dir
    }

    pub fn load() -> Self {
        match std::fs::read_to_string(Self::path()) {
            Ok(text) => Self::from_json(&text).unwrap_or_else(|e| {
                log::warn!("config parse failed ({e}); using defaults");
                Self::default()
            }),
            Err(_) => Self::default(),
        }
    }

    pub fn from_json(text: &str) -> Result<Self, serde_json::Error> {
        let mut cfg: AppConfig = serde_json::from_str(text)?;
        cfg.normalize();
        Ok(cfg)
    }

    /// Migrate old formats and repair invariants (at least one profile, unique ids, valid index).
    fn normalize(&mut self) {
        if let Some(old) = self.clickers.take() {
            // A v0.1 file has no "profiles" key, so serde filled in the default profile.
            self.profiles = vec![Profile { name: "Default".into(), clickers: old }];
            self.active_profile = 0;
        }
        if self.profiles.is_empty() {
            self.profiles.push(Profile { name: "Default".into(), clickers: vec![] });
        }
        if self.active_profile >= self.profiles.len() {
            self.active_profile = 0;
        }
        let mut seen = std::collections::HashSet::new();
        let mut max = 0;
        for p in &mut self.profiles {
            for c in &mut p.clickers {
                c.migrate();
                max = max.max(c.id);
            }
        }
        if self.next_id <= max {
            self.next_id = max + 1;
        }
        // Re-id duplicates (would confuse the engine, which is keyed by id).
        let mut next = self.next_id;
        for p in &mut self.profiles {
            for c in &mut p.clickers {
                if !seen.insert(c.id) || c.id == 0 {
                    c.id = next;
                    next += 1;
                    seen.insert(c.id);
                }
            }
        }
        self.next_id = next;
        self.font_size = self.font_size.clamp(11.0, 22.0);
        if let Some(dark) = self.dark_theme.take() {
            self.theme = if dark { ThemePref::Dark } else { ThemePref::Light };
        }
        if self.lang.is_none() {
            self.lang = Some(Lang::system_default());
        }
    }

    pub fn lang(&self) -> Lang {
        self.lang.unwrap_or_default()
    }

    pub fn save(&self) {
        let path = Self::path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        match serde_json::to_string_pretty(self) {
            Ok(text) => {
                // Write-then-rename so a crash mid-write never leaves a truncated config.
                let tmp = path.with_extension("json.tmp");
                if std::fs::write(&tmp, text).is_ok() {
                    if let Err(e) = std::fs::rename(&tmp, &path) {
                        log::error!("config save failed: {e}");
                    }
                }
            }
            Err(e) => log::error!("config serialize failed: {e}"),
        }
    }

    pub fn alloc_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    pub fn profile(&self) -> &Profile {
        &self.profiles[self.active_profile]
    }

    pub fn clickers(&self) -> &Vec<ClickerConfig> {
        &self.profiles[self.active_profile].clickers
    }

    pub fn clickers_mut(&mut self) -> &mut Vec<ClickerConfig> {
        &mut self.profiles[self.active_profile].clickers
    }

    /// Copy a profile, giving every clicker a fresh id.
    pub fn duplicate_profile(&mut self, idx: usize) -> usize {
        let mut p = self.profiles[idx].clone();
        p.name = format!("{} copy", p.name);
        for c in &mut p.clickers {
            c.id = self.alloc_id();
            c.hotkey = Hotkey::NONE; // the same key cannot be registered twice
        }
        self.profiles.push(p);
        self.profiles.len() - 1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_point_scales() {
        let p = ClickPoint { x: 50, y: 20, client_w: 100, client_h: 40, relative: true };
        assert_eq!(p.resolve(200, 80), (100, 40));
        assert_eq!(p.resolve(0, 0), (50, 20));
    }

    #[test]
    fn absolute_point_unchanged() {
        let p = ClickPoint { x: 50, y: 20, client_w: 100, client_h: 40, relative: false };
        assert_eq!(p.resolve(200, 80), (50, 20));
    }

    #[test]
    fn migrates_v01_config() {
        let old = r#"{
          "clickers": [{"id": 3, "name": "Old", "target": {"title": "t", "class": "c", "process": "p.exe"},
            "point": {"x": 5, "y": 6, "client_w": 10, "client_h": 12, "relative": true},
            "button": "Right", "kind": "Double", "mode": "Background", "post_to_root": false,
            "interval_ms": 250, "jitter_ms": 0, "jitter_px": 0, "max_clicks": 0, "max_seconds": 0, "auto_run": true}],
          "next_id": 4, "autostart": false, "start_minimized": false, "close_to_tray": true,
          "hotkey_toggle_all": 117, "hotkey_stop_all": 119, "dark_theme": true }"#;
        let cfg = AppConfig::from_json(old).unwrap();
        assert_eq!(cfg.profiles.len(), 1);
        assert_eq!(cfg.profiles[0].name, "Default");
        let c = &cfg.clickers()[0];
        assert_eq!(c.id, 3);
        assert_eq!(c.kind(), ClickerKind::Simple);
        assert_eq!(cfg.theme, ThemePref::Dark);
        assert!(c.auto_run);
        assert_eq!(c.steps.len(), 1);
        assert_eq!(c.steps[0].wait_ms, 250);
        match &c.steps[0].action {
            Action::Click { point, button, kind } => {
                assert_eq!((point.x, point.y), (5, 6));
                assert_eq!(*button, MouseButton::Right);
                assert_eq!(*kind, ClickKind::Double);
            }
            other => panic!("unexpected {other:?}"),
        }
        assert_eq!(cfg.hotkey_toggle_all, Hotkey::key(0x75));
        assert_eq!(cfg.hotkey_stop_all.label(), "F8");
        // Round-trips without the legacy fields.
        let json = serde_json::to_string(&cfg).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(v.get("clickers").is_none(), "legacy top-level list must not be written");
        let clicker = &v["profiles"][0]["clickers"][0];
        for legacy in ["point", "button", "kind", "interval_ms"] {
            assert!(clicker.get(legacy).is_none(), "legacy field {legacy} written back");
        }
        let again = AppConfig::from_json(&json).unwrap();
        assert_eq!(again, cfg);
    }

    #[test]
    fn hotkey_labels_and_duplicate_ids() {
        let h = Hotkey { vk: 0x41, ctrl: true, alt: false, shift: true };
        assert_eq!(h.label(), "Ctrl+Shift+A");
        let mut cfg = AppConfig::default();
        cfg.clickers_mut()[0].hotkey = Hotkey::key(0x70);
        let i = cfg.duplicate_profile(0);
        let a = cfg.profiles[0].clickers[0].id;
        let b = cfg.profiles[i].clickers[0].id;
        assert_ne!(a, b);
        assert!(!cfg.profiles[i].clickers[0].hotkey.is_set());
    }
}
