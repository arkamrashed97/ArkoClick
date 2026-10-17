//! Classic clicker tests. The ones that send real mouse clicks at the current cursor
//! position are `#[ignore]`d so `cargo test` never clicks on whatever is on screen; run them
//! deliberately with `cargo test --test classic_click -- --ignored`.

use arkoclick::config::{find_available_classic_hotkey, ClickerConfig, ClickerKind, Hotkey, Lang, Step};
use arkoclick::engine::{Engine, Status};
use std::time::Duration;

#[test]
fn classic_clicker_defaults_and_naming() {
    let c_en = ClickerConfig::new_of_kind(1, ClickerKind::Classic, 1, Lang::En);
    assert_eq!(c_en.kind(), ClickerKind::Classic);
    assert_eq!(c_en.name, "Classic 1");
    assert_eq!(c_en.hotkey, Hotkey::key(0x76)); // F7
    assert_eq!(c_en.hotkey.label(), "F7");
    assert!(!c_en.target.is_set());

    let c_ar = ClickerConfig::new_of_kind(2, ClickerKind::Classic, 1, Lang::Ar);
    assert_eq!(c_ar.kind(), ClickerKind::Classic);
    assert_eq!(c_ar.name, "كلاسيكي 1");
    assert_eq!(c_ar.hotkey, Hotkey::key(0x76));
}

#[test]
fn classic_hotkey_fallback() {
    let used = vec![Hotkey::key(0x76)]; // F7 is taken
    let next = find_available_classic_hotkey(&used);
    assert_eq!(next, Hotkey::key(0x78)); // F9

    let used_both = vec![Hotkey::key(0x76), Hotkey::key(0x78)];
    let next_after = find_available_classic_hotkey(&used_both);
    assert_eq!(next_after, Hotkey::key(0x79)); // F10
}

#[test]
#[ignore = "sends real mouse clicks wherever the cursor is"]
fn classic_clicker_runs_without_target_window() {
    let engine = Engine::new();
    let mut cfg = ClickerConfig::new_of_kind(10, ClickerKind::Classic, 1, Lang::En);
    cfg.steps = vec![Step::click(Default::default(), 10)];
    cfg.max_rounds = 3;

    assert!(!cfg.target.is_set(), "classic clicker has no target window");

    engine.sync(std::slice::from_ref(&cfg));
    engine.start(10);

    // Give it a moment to run its 3 clicks
    std::thread::sleep(Duration::from_millis(150));

    let stats = engine.stats();
    let st = &stats[&10];
    assert!(st.target_found, "classic clicker target is always considered valid/found");
    // In an interactive desktop session, SendInput succeeds and status reaches Finished with count >= 3.
    // In a non-interactive/locked environment, Windows security rejects SendInput with Access Denied,
    // so status is Status::Waiting("SendInput rejected").
    // Either way, it must NOT fail with "no target window selected"!
    assert_ne!(st.status, Status::Error("no target window selected".into()));
    assert!(
        st.status == Status::Finished || matches!(&st.status, Status::Waiting(msg) if msg == "SendInput rejected"),
        "unexpected status: {:?}",
        st.status
    );
}

#[test]
#[ignore = "sends a real mouse click wherever the cursor is"]
fn classic_test_step_succeeds() {
    let engine = Engine::new();
    let cfg = ClickerConfig::new_of_kind(20, ClickerKind::Classic, 1, Lang::En);
    engine.sync(std::slice::from_ref(&cfg));

    let res = engine.test_step(20, 0);
    // On interactive desktop: Ok(()). On locked/headless desktop: Err("SendInput rejected").
    // Neither should report "no target window selected" or "no such step".
    match res {
        Ok(()) => {}
        Err(e) => assert!(e == "SendInput rejected" || e == "cursor is over ArkoClick", "{e}"),
    }
}

#[test]
fn start_all_never_starts_classic_clickers() {
    // No real input: the classic task must stay idle, and the simple one has no window.
    let engine = Engine::new();
    let classic = ClickerConfig::new_of_kind(30, ClickerKind::Classic, 1, Lang::En);
    let simple = ClickerConfig::new_of_kind(31, ClickerKind::Simple, 1, Lang::En);
    engine.sync(&[classic, simple]);
    engine.start_all();
    std::thread::sleep(Duration::from_millis(50));
    let st = engine.stats();
    assert_eq!(st[&30].status, Status::Idle, "classic must not start from start-all");
    assert_eq!(st[&30].count, 0);
    assert!(matches!(st[&31].status, Status::Error(_)), "simple task without a window reports an error");
    // toggle_all with nothing running behaves like start_all.
    engine.toggle_all();
    assert_eq!(engine.stats()[&30].status, Status::Idle);
}

#[test]
fn classic_countdown_can_be_cancelled_before_any_click() {
    let engine = Engine::new();
    let cfg = ClickerConfig::new_of_kind(40, ClickerKind::Classic, 1, Lang::En);
    engine.sync(std::slice::from_ref(&cfg));
    engine.start_after(40, Duration::from_secs(3));
    std::thread::sleep(Duration::from_millis(100));
    let st = &engine.stats()[&40];
    assert_eq!(st.status, Status::Running);
    let left = st.starts_in.expect("countdown is reported");
    assert!(left > Duration::from_millis(2500) && left <= Duration::from_secs(3), "{left:?}");
    engine.stop(40);
    std::thread::sleep(Duration::from_millis(50));
    let st = &engine.stats()[&40];
    assert_eq!(st.status, Status::Idle);
    assert_eq!(st.count, 0, "stopped during the countdown: no click was sent");
    assert_eq!(st.starts_in, None);
}

#[test]
fn starter_profile_does_not_grab_f7() {
    let cfg = arkoclick::config::AppConfig::default();
    let classic = cfg.clickers().iter().find(|c| c.kind() == ClickerKind::Classic).expect("starter classic task");
    assert!(!classic.hotkey.is_set(), "no global shortcut taken on first launch");
}

/// Puts a large window owned by this (test) process under the mouse cursor and starts a
/// classic task: it must skip every beat instead of clicking. If the mouse is moved off the
/// window during the 300 ms run, a real click could land elsewhere, hence `#[ignore]`.
#[test]
#[ignore = "places a window under the real cursor; run deliberately"]
fn classic_never_clicks_on_own_window() {
    use arkoclick::win::{screen, to_wide, window};
    use windows::core::PCWSTR;
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::WindowsAndMessaging::*;

    let (cx, cy) = screen::cursor_pos();
    let hwnd = unsafe {
        let class = to_wide("Static");
        CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            PCWSTR(class.as_ptr()),
            PCWSTR::null(),
            // SS_NOTIFY (0x100): without it a Static control is transparent to hit-testing.
            WS_POPUP | WS_VISIBLE | WINDOW_STYLE(0x100),
            cx - 400,
            cy - 400,
            800,
            800,
            None,
            None,
            GetModuleHandleW(None).unwrap(),
            None,
        )
        .expect("window")
    };
    std::thread::sleep(Duration::from_millis(100));
    assert!(window::cursor_over_own_process(), "test window should be under the cursor");

    let engine = Engine::new();
    let mut cfg = ClickerConfig::new_of_kind(50, ClickerKind::Classic, 1, Lang::En);
    cfg.steps = vec![Step::click(Default::default(), 10)];
    engine.sync(std::slice::from_ref(&cfg));
    engine.start(50);
    std::thread::sleep(Duration::from_millis(300));
    let st = engine.stats()[&50].clone();
    engine.stop_all();
    unsafe {
        let _ = DestroyWindow(hwnd);
    }
    assert_eq!(st.status, Status::Running, "keeps running while skipping");
    assert_eq!(st.count, 0, "no click may be sent while the cursor is over ArkoClick");
}
