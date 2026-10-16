//! Measures how many background clicks / keys a Chromium window really receives while it
//! is visible, minimized, or hidden with ArkoClick's "Hide window (keep running)".
//!
//! Open `test-target/ArkoClick-TestTarget.html` in Chrome or Edge first (the page mirrors
//! its counters into the window title), then run:
//!     cargo run --release --example chrome_probe
//! Nothing touches the real mouse or keyboard.

use arkoclick::config::{Action, ClickPoint, ClickerConfig, Step, TargetInfo};
use arkoclick::engine::Engine;
use arkoclick::win::{ghost, window, SafeHwnd};
use std::time::Duration;
use windows::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_MINIMIZE, SW_RESTORE};

const N: u64 = 40;
const WAIT_MS: u64 = 50;

fn typed(h: SafeHwnd) -> u64 {
    let t = window::title_of(h);
    t.split("typed ").nth(1).and_then(|r| r.split(|c: char| !c.is_ascii_digit()).next()).and_then(|n| n.parse().ok()).unwrap_or(0)
}

fn counters(h: SafeHwnd) -> (u64, u64) {
    // "ArkoClick Test Target | clicks N | target T | keys K - Google Chrome"
    let t = window::title_of(h);
    let num = |key: &str| {
        t.split(key).nth(1).and_then(|r| r.trim().split(|c: char| !c.is_ascii_digit()).next()).and_then(|n| n.parse().ok()).unwrap_or(0)
    };
    (num("clicks "), num("keys "))
}

fn find_page() -> Option<SafeHwnd> {
    window::enumerate().into_iter().find(|w| w.title.starts_with("ArkoClick Test Target")).map(|w| w.hwnd)
}

fn run(engine: &Engine, cfg: &mut ClickerConfig, action: Action) {
    cfg.steps = vec![Step::new(action, WAIT_MS)];
    cfg.max_rounds = N;
    engine.sync(std::slice::from_ref(cfg));
    engine.start(cfg.id);
    std::thread::sleep(Duration::from_millis(N * WAIT_MS + 800));
    engine.stop_all();
    std::thread::sleep(Duration::from_millis(400));
}

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let Some(h) = find_page() else {
        eprintln!("Open test-target/ArkoClick-TestTarget.html in Chrome/Edge first.");
        std::process::exit(1);
    };
    let info = window::info_of(h);
    let (w, hgt) = window::client_size(h).unwrap_or((800, 600));
    println!("target: {} [{}] client {}x{}", info.title, info.class, w, hgt);
    let mut cfg = ClickerConfig::new(1);
    cfg.target = TargetInfo { title: info.title.clone(), class: info.class.clone(), process: info.process.clone(), hwnd: Some(h.0) };
    // Lower-middle of the page: inside the content area, away from links/inputs.
    let click = Action::Click {
        point: ClickPoint { x: w / 2, y: hgt * 3 / 4, client_w: w, client_h: hgt, relative: true },
        button: Default::default(),
        kind: Default::default(),
    };
    let key = Action::Key { vk: 0x41 };
    let engine = Engine::new();

    let mut report = Vec::new();
    // Text steps first, while the page's text box still has focus (it autofocuses).
    let text = "مرحبا ArkoClick 123";
    let per = text.chars().count() as u64;
    for (label, hidden) in [("text, visible", false), ("text, hidden", true)] {
        if hidden {
            ghost::hide(h).expect("hide");
            std::thread::sleep(Duration::from_millis(600));
        }
        let t0 = typed(h);
        cfg.steps = vec![Step::new(Action::Text { text: text.into() }, 100)];
        cfg.max_rounds = 5;
        engine.sync(std::slice::from_ref(&cfg));
        engine.start(cfg.id);
        std::thread::sleep(Duration::from_millis(1500));
        engine.stop_all();
        if hidden {
            ghost::show(h);
        }
        std::thread::sleep(Duration::from_millis(500));
        report.push(format!("{label:<24} typed {:>3}/{} characters", typed(h) - t0, 5 * per));
    }
    for (label, prep, undo) in [
        ("visible", 0, 0),
        ("minimized", 1, 1),
        ("hidden (ArkoClick)", 2, 2),
        ("hidden, was minimized", 3, 2),
    ] {
        match prep {
            1 => unsafe {
                let _ = ShowWindow(h.hwnd(), SW_MINIMIZE);
            },
            2 => ghost::hide(h).expect("hide"),
            3 => {
                unsafe {
                    let _ = ShowWindow(h.hwnd(), SW_MINIMIZE);
                }
                std::thread::sleep(Duration::from_millis(500));
                ghost::hide(h).expect("hide");
            }
            _ => {}
        }
        std::thread::sleep(Duration::from_millis(600));
        let (c0, k0) = counters(h);
        // The raw "minimized" row measures Chrome itself, so the auto-hide helper is off there.
        cfg.hide_when_minimized = prep != 1;
        run(&engine, &mut cfg, click.clone());
        run(&engine, &mut cfg, key.clone());
        match undo {
            1 => unsafe {
                let _ = ShowWindow(h.hwnd(), SW_RESTORE);
            },
            2 => ghost::show(h),
            _ => {}
        }
        std::thread::sleep(Duration::from_millis(700));
        let (c1, k1) = counters(h);
        report.push(format!("{label:<24} clicks {:>2}/{N}   keys {:>2}/{N}", c1 - c0, k1 - k0));
    }

    // The real-life case: the user minimizes the browser in the middle of a run, with the
    // window watcher active exactly as in the app.
    arkoclick::win::watch::spawn(std::sync::Arc::clone(&engine));
    std::thread::sleep(Duration::from_millis(300));
    let (c0, _) = counters(h);
    cfg.hide_when_minimized = true;
    cfg.steps = vec![Step::new(click.clone(), WAIT_MS)];
    cfg.max_rounds = N;
    engine.sync(std::slice::from_ref(&cfg));
    engine.start(cfg.id);
    std::thread::sleep(Duration::from_millis(300));
    unsafe {
        let _ = ShowWindow(h.hwnd(), SW_MINIMIZE);
    }
    std::thread::sleep(Duration::from_millis(600));
    let hidden_during = ghost::is_ghosted(h) && !window::is_iconic(h);
    std::thread::sleep(Duration::from_millis(N * WAIT_MS + 200));
    let iconic_after = window::is_iconic(h);
    unsafe {
        let _ = ShowWindow(h.hwnd(), SW_RESTORE);
    }
    std::thread::sleep(Duration::from_millis(700));
    let (c1, _) = counters(h);
    report.push(format!(
        "{:<24} clicks {:>2}/{N}   (auto-hidden: {hidden_during}, minimized again after stop: {iconic_after})",
        "minimized mid-run (auto)",
        c1 - c0
    ));
    for r in report {
        println!("{r}");
    }
}
