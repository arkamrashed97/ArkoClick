//! End-to-end checks of the core promises against real Win32 windows: a clicker bound to
//! a window delivers clicks / keys / text to the right control at the right coordinates,
//! keeps doing so while the window is minimized or hidden, runs multi-step sequences in
//! order, honours limits and re-binds when the window is recreated.

use std::sync::atomic::{AtomicI32, AtomicIsize, AtomicU32, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use arkoclick::config::{Action, ClickKind, ClickPoint, ClickerConfig, MouseButton, Step, TargetInfo};
use arkoclick::engine::{Engine, Status};
use arkoclick::win::{ghost, to_wide, window, SafeHwnd};
use windows::core::PCWSTR;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::SetFocus;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetWindowLongPtrW, PeekMessageW, RegisterClassW,
    ShowWindow, TranslateMessage, CW_USEDEFAULT, GWL_EXSTYLE, MSG, PM_REMOVE, SW_MINIMIZE, SW_SHOWNOACTIVATE,
    WINDOW_EX_STYLE, WM_CHAR, WM_KEYDOWN, WM_KEYUP, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_RBUTTONDOWN, WNDCLASSW, WS_CHILD,
    WS_EX_LAYERED, WS_EX_TRANSPARENT, WS_OVERLAPPEDWINDOW, WS_VISIBLE,
};

// Tests in one binary run in parallel by default; the counters are process-global, so
// serialize the tests with a mutex.
static SERIAL: Mutex<()> = Mutex::new(());

static CHILD_LDOWN: AtomicU32 = AtomicU32::new(0);
static CHILD_LUP: AtomicU32 = AtomicU32::new(0);
static ROOT_LDOWN: AtomicU32 = AtomicU32::new(0);
static CHILD_RDOWN: AtomicU32 = AtomicU32::new(0);
static CHILD_KEYDOWN: AtomicU32 = AtomicU32::new(0);
static CHILD_KEYUP: AtomicU32 = AtomicU32::new(0);
static LAST_X: AtomicI32 = AtomicI32::new(-1);
static LAST_Y: AtomicI32 = AtomicI32::new(-1);
static LAST_VK: AtomicU32 = AtomicU32::new(0);
static CHILD_HWND: AtomicIsize = AtomicIsize::new(0);
/// Order of events as a compact string: 'c' child click, 'r' root click, 'k' child key.
static ORDER: Mutex<String> = Mutex::new(String::new());
static TYPED: Mutex<String> = Mutex::new(String::new());
static PENDING_SURROGATE: AtomicU32 = AtomicU32::new(0);

unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let is_child = hwnd.0 as isize == CHILD_HWND.load(Ordering::Relaxed);
    match msg {
        WM_LBUTTONDOWN => {
            if is_child {
                CHILD_LDOWN.fetch_add(1, Ordering::Relaxed);
                LAST_X.store((lparam.0 & 0xFFFF) as i16 as i32, Ordering::Relaxed);
                LAST_Y.store(((lparam.0 >> 16) & 0xFFFF) as i16 as i32, Ordering::Relaxed);
                ORDER.lock().unwrap().push('c');
            } else {
                ROOT_LDOWN.fetch_add(1, Ordering::Relaxed);
                ORDER.lock().unwrap().push('r');
            }
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            if is_child {
                CHILD_LUP.fetch_add(1, Ordering::Relaxed);
            }
            LRESULT(0)
        }
        WM_RBUTTONDOWN => {
            if is_child {
                CHILD_RDOWN.fetch_add(1, Ordering::Relaxed);
            }
            LRESULT(0)
        }
        WM_KEYDOWN if is_child => {
            CHILD_KEYDOWN.fetch_add(1, Ordering::Relaxed);
            LAST_VK.store(wparam.0 as u32, Ordering::Relaxed);
            ORDER.lock().unwrap().push('k');
            LRESULT(0)
        }
        WM_KEYUP if is_child => {
            CHILD_KEYUP.fetch_add(1, Ordering::Relaxed);
            LRESULT(0)
        }
        WM_CHAR if is_child => {
            let unit = wparam.0 as u32;
            let mut t = TYPED.lock().unwrap();
            if (0xD800..0xDC00).contains(&unit) {
                PENDING_SURROGATE.store(unit, Ordering::Relaxed);
            } else if (0xDC00..0xE000).contains(&unit) {
                let hi = PENDING_SURROGATE.swap(0, Ordering::Relaxed);
                if let Some(c) = char::decode_utf16([hi as u16, unit as u16]).next().and_then(Result::ok) {
                    t.push(c);
                }
            } else if let Some(c) = char::from_u32(unit) {
                t.push(c);
            }
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

struct TestWindow {
    root: HWND,
    child: HWND,
}

impl TestWindow {
    fn create() -> Self {
        unsafe {
            let class = to_wide("ArkoClickTestTarget");
            let hinst = GetModuleHandleW(None).unwrap();
            let wc = WNDCLASSW {
                lpfnWndProc: Some(wnd_proc),
                hInstance: hinst.into(),
                lpszClassName: PCWSTR(class.as_ptr()),
                ..Default::default()
            };
            RegisterClassW(&wc); // fails harmlessly when already registered
            let title = to_wide("ArkoClick test target");
            let root = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                PCWSTR(class.as_ptr()),
                PCWSTR(title.as_ptr()),
                WS_OVERLAPPEDWINDOW,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                400,
                300,
                None,
                None,
                hinst,
                None,
            )
            .expect("create root");
            // One child control at (150,100) with size 200x150 inside the client area.
            let child = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                PCWSTR(class.as_ptr()),
                PCWSTR::null(),
                WS_CHILD | WS_VISIBLE,
                150,
                100,
                200,
                150,
                root,
                None,
                hinst,
                None,
            )
            .expect("create child");
            CHILD_HWND.store(child.0 as isize, Ordering::Relaxed);
            let _ = ShowWindow(root, SW_SHOWNOACTIVATE);
            Self { root, child }
        }
    }

    fn pump_for(&self, d: Duration) {
        let end = Instant::now() + d;
        let mut msg = MSG::default();
        while Instant::now() < end {
            unsafe {
                while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    }
}

impl Drop for TestWindow {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyWindow(self.root);
        }
    }
}

fn reset_counters() {
    for c in [&CHILD_LDOWN, &CHILD_LUP, &ROOT_LDOWN, &CHILD_RDOWN, &CHILD_KEYDOWN, &CHILD_KEYUP, &LAST_VK] {
        c.store(0, Ordering::Relaxed);
    }
    ORDER.lock().unwrap().clear();
    TYPED.lock().unwrap().clear();
}

fn point_for(root: HWND, x: i32, y: i32) -> ClickPoint {
    let (w, h) = window::client_size(SafeHwnd::from_hwnd(root)).unwrap();
    ClickPoint { x, y, client_w: w, client_h: h, relative: true }
}

fn clicker_for(root: HWND, steps: Vec<Step>) -> ClickerConfig {
    let sh = SafeHwnd::from_hwnd(root);
    let info = window::info_of(sh);
    let mut c = ClickerConfig::new(1);
    c.target = TargetInfo { title: info.title, class: info.class, process: info.process, hwnd: Some(sh.0) };
    c.steps = steps;
    c
}

fn click_step(root: HWND, x: i32, y: i32, wait_ms: u64) -> Step {
    Step::click(point_for(root, x, y), wait_ms)
}

#[test]
fn clicks_reach_child_control_with_translated_coordinates() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let win = TestWindow::create();
    reset_counters();
    let engine = Engine::new();
    // Root client point (200,150) lies inside the child, at child-local (50,50).
    let cfg = clicker_for(win.root, vec![click_step(win.root, 200, 150, 10)]);
    engine.sync(std::slice::from_ref(&cfg));
    engine.start(1);
    win.pump_for(Duration::from_millis(400));
    engine.stop_all();
    win.pump_for(Duration::from_millis(50));

    let down = CHILD_LDOWN.load(Ordering::Relaxed);
    let up = CHILD_LUP.load(Ordering::Relaxed);
    assert!(down >= 10, "expected many clicks on the child, got {down}");
    assert_eq!(down, up, "every down must be paired with an up");
    assert_eq!(ROOT_LDOWN.load(Ordering::Relaxed), 0, "clicks must go to the child, not the root");
    assert_eq!((LAST_X.load(Ordering::Relaxed), LAST_Y.load(Ordering::Relaxed)), (50, 50), "child-relative coords");
    assert_eq!(engine.stats()[&1].count as u32, down, "engine count matches delivered clicks");
}

#[test]
fn clicks_keep_arriving_while_minimized() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let win = TestWindow::create();
    let engine = Engine::new();
    let mut cfg = clicker_for(win.root, vec![click_step(win.root, 200, 150, 10)]);
    // Raw delivery into a minimized window, without the auto-hide helper.
    cfg.hide_when_minimized = false;
    engine.sync(std::slice::from_ref(&cfg));
    engine.start(1);
    // Let the engine resolve the child while the window is still visible...
    win.pump_for(Duration::from_millis(100));
    reset_counters();
    // ...then minimize and make sure clicks still land on the child.
    unsafe {
        let _ = ShowWindow(win.root, SW_MINIMIZE);
    }
    assert!(window::is_iconic(SafeHwnd::from_hwnd(win.root)));
    win.pump_for(Duration::from_millis(400));
    engine.stop_all();
    let down = CHILD_LDOWN.load(Ordering::Relaxed);
    assert!(down >= 10, "expected clicks while minimized, got {down}");
    assert_eq!(ROOT_LDOWN.load(Ordering::Relaxed), 0);
}

#[test]
fn right_button_and_action_limit() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let win = TestWindow::create();
    reset_counters();
    let engine = Engine::new();
    let step = Step::new(
        Action::Click { point: point_for(win.root, 200, 150), button: MouseButton::Right, kind: ClickKind::Single },
        10,
    );
    let mut cfg = clicker_for(win.root, vec![step]);
    cfg.max_rounds = 5;
    engine.sync(std::slice::from_ref(&cfg));
    engine.start(1);
    win.pump_for(Duration::from_millis(300));
    assert_eq!(CHILD_RDOWN.load(Ordering::Relaxed), 5, "limit must stop exactly at 5");
    assert_eq!(CHILD_LDOWN.load(Ordering::Relaxed), 0);
    assert_eq!(engine.stats()[&1].status, Status::Finished);
}

#[test]
fn sequence_runs_steps_in_order() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let win = TestWindow::create();
    reset_counters();
    unsafe {
        let _ = SetFocus(win.child);
    }
    let engine = Engine::new();
    // child click -> root click (outside the child) -> key A, repeated twice.
    let mut cfg = clicker_for(
        win.root,
        vec![
            click_step(win.root, 200, 150, 15),
            click_step(win.root, 20, 20, 15),
            Step::new(Action::Key { vk: 0x41 }, 15),
        ],
    );
    cfg.max_rounds = 2;
    engine.sync(std::slice::from_ref(&cfg));
    engine.start(1);
    win.pump_for(Duration::from_millis(500));
    assert_eq!(engine.stats()[&1].status, Status::Finished);
    assert_eq!(*ORDER.lock().unwrap(), "crkcrk", "steps must run in order and loop");
    assert_eq!(CHILD_KEYDOWN.load(Ordering::Relaxed), 2);
    assert_eq!(CHILD_KEYUP.load(Ordering::Relaxed), 2);
    assert_eq!(LAST_VK.load(Ordering::Relaxed), 0x41);
    // The target's own TranslateMessage turned each posted key into one character
    // (which character depends on the active keyboard layout, e.g. "aa" or "شش").
    let typed = TYPED.lock().unwrap().clone();
    assert_eq!(typed.chars().count(), 2, "typed {typed:?}");
}

#[test]
fn text_step_types_unicode_into_focused_control() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let win = TestWindow::create();
    reset_counters();
    unsafe {
        let _ = SetFocus(win.child);
    }
    let engine = Engine::new();
    let text = "Hi مرحبا 👋";
    let mut cfg = clicker_for(win.root, vec![Step::new(Action::Text { text: text.into() }, 10)]);
    cfg.max_rounds = 1;
    engine.sync(std::slice::from_ref(&cfg));
    engine.start(1);
    win.pump_for(Duration::from_millis(200));
    assert_eq!(*TYPED.lock().unwrap(), text);
}

#[test]
fn hidden_window_keeps_receiving_and_is_restored() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let win = TestWindow::create();
    let h = SafeHwnd::from_hwnd(win.root);
    let ex_before = unsafe { GetWindowLongPtrW(win.root, GWL_EXSTYLE) } as u32;
    // Hiding a minimized window must un-minimize it (that is the whole point).
    unsafe {
        let _ = ShowWindow(win.root, SW_MINIMIZE);
    }
    ghost::hide(h).expect("hide");
    win.pump_for(Duration::from_millis(50));
    assert!(ghost::is_ghosted(h));
    assert!(!window::is_iconic(h), "ghosted window must not stay minimized");
    let ex = unsafe { GetWindowLongPtrW(win.root, GWL_EXSTYLE) } as u32;
    assert!(ex & WS_EX_LAYERED.0 != 0 && ex & WS_EX_TRANSPARENT.0 != 0);

    reset_counters();
    let engine = Engine::new();
    let cfg = clicker_for(win.root, vec![click_step(win.root, 200, 150, 10)]);
    engine.sync(std::slice::from_ref(&cfg));
    engine.start(1);
    win.pump_for(Duration::from_millis(300));
    engine.stop_all();
    let st = &engine.stats()[&1];
    assert!(
        CHILD_LDOWN.load(Ordering::Relaxed) >= 10,
        "clicks while hidden: child={} root={} status={:?} count={} size={:?} iconic={}",
        CHILD_LDOWN.load(Ordering::Relaxed),
        ROOT_LDOWN.load(Ordering::Relaxed),
        st.status,
        st.count,
        st.last_client_size,
        window::is_iconic(h)
    );

    ghost::show(h);
    assert!(!ghost::is_ghosted(h));
    let ex_after = unsafe { GetWindowLongPtrW(win.root, GWL_EXSTYLE) } as u32;
    assert_eq!(ex_after, ex_before, "original extended style restored");
}

#[test]
fn rebinds_after_target_is_recreated() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let engine = Engine::new();
    {
        let win = TestWindow::create();
        let cfg = clicker_for(win.root, vec![click_step(win.root, 200, 150, 10)]);
        engine.sync(std::slice::from_ref(&cfg));
        engine.start(1);
        win.pump_for(Duration::from_millis(100));
    } // window destroyed here
    std::thread::sleep(Duration::from_millis(150));
    let st = engine.stats();
    assert!(matches!(st[&1].status, Status::Waiting(_)), "should be waiting, got {:?}", st[&1].status);

    // Recreate a window with the same class + process: the clicker must find it on its own.
    let win2 = TestWindow::create();
    reset_counters();
    win2.pump_for(Duration::from_millis(1500));
    engine.stop_all();
    assert!(CHILD_LDOWN.load(Ordering::Relaxed) > 0, "clicker did not re-bind to the new window");
}

#[test]
fn minimize_becomes_hidden_while_running_and_minimized_again_on_stop() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let win = TestWindow::create();
    let h = SafeHwnd::from_hwnd(win.root);
    let ex_before = unsafe { GetWindowLongPtrW(win.root, GWL_EXSTYLE) } as u32;
    let engine = Engine::new();
    let cfg = clicker_for(win.root, vec![click_step(win.root, 200, 150, 10)]);
    assert!(cfg.hide_when_minimized, "on by default");
    engine.sync(std::slice::from_ref(&cfg));
    engine.start(1);
    win.pump_for(Duration::from_millis(100));

    // The user minimizes the target while the clicker runs...
    unsafe {
        let _ = ShowWindow(win.root, SW_MINIMIZE);
    }
    win.pump_for(Duration::from_millis(200));
    // ...ArkoClick turns that into "hidden but running".
    assert!(ghost::is_ghosted(h) && ghost::is_auto(h), "auto-hidden");
    assert!(!window::is_iconic(h), "not minimized any more");
    reset_counters();
    win.pump_for(Duration::from_millis(300));
    assert!(CHILD_LDOWN.load(Ordering::Relaxed) >= 10, "clicks keep landing on the child");

    // Stopping the clicker puts the window back the way the user left it.
    engine.stop_all();
    win.pump_for(Duration::from_millis(100));
    assert!(!ghost::is_ghosted(h));
    assert!(window::is_iconic(h), "minimized again");
    let ex_after = unsafe { GetWindowLongPtrW(win.root, GWL_EXSTYLE) } as u32;
    assert_eq!(ex_after, ex_before, "styles restored");
}

#[test]
fn switched_off_steps_are_skipped() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let win = TestWindow::create();
    reset_counters();
    let engine = Engine::new();
    // child click (off) -> root click (on): only root clicks may arrive.
    let mut off = click_step(win.root, 200, 150, 10);
    off.enabled = false;
    let mut cfg = clicker_for(win.root, vec![off, click_step(win.root, 20, 20, 10)]);
    cfg.max_rounds = 4;
    engine.sync(std::slice::from_ref(&cfg));
    engine.start(1);
    win.pump_for(Duration::from_millis(300));
    assert_eq!(engine.stats()[&1].status, Status::Finished);
    assert_eq!(*ORDER.lock().unwrap(), "rrrr");
    assert_eq!(engine.stats()[&1].rounds, 4);
}
