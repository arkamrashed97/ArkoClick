//! Click engine: one scheduler thread drives every running clicker from a min-heap of
//! deadlines. No per-clicker threads, no busy loops: the thread sleeps on a condvar until
//! the earliest deadline or until the UI/hotkeys change something.
//!
//! Each clicker runs its steps in order (click / key / text), waiting each step's
//! `wait_ms` before the next, and loops.

use crate::config::{Action, ClickMode, ClickerConfig};
use crate::win::{ghost, input, timer::HighResTimer, window, SafeHwnd};
use rand::Rng;
use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

/// Below this wait we hold the 1 ms multimedia timer while running.
const HIRES_THRESHOLD_MS: u64 = 20;
/// How often to retry finding a target window that is gone.
const REBIND_EVERY: Duration = Duration::from_millis(750);
/// Re-resolve the child control under the point this often (layouts change).
const CHILD_CACHE_TTL: Duration = Duration::from_secs(2);
/// After a failed automatic hide, do not retry for this long.
const AUTOHIDE_RETRY: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    Idle,
    Running,
    /// Target window not found; still scheduled and will resume once it appears.
    Waiting(String),
    /// Hit the action/time limit.
    Finished,
    Error(String),
}

impl Status {
    pub fn is_active(&self) -> bool {
        matches!(self, Status::Running | Status::Waiting(_))
    }
    pub fn label(&self) -> String {
        match self {
            Status::Idle => "Idle".into(),
            Status::Running => "Running".into(),
            Status::Waiting(w) => format!("Waiting: {w}"),
            Status::Finished => "Finished".into(),
            Status::Error(e) => format!("Error: {e}"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ClickerStats {
    pub status: Status,
    /// Actions delivered (clicks, key presses, texts).
    pub count: u64,
    /// Completed passes over the whole sequence.
    pub rounds: u64,
    pub elapsed: Duration,
    pub target_found: bool,
    /// The live window handle the clicker is bound to (may differ from the picked one
    /// after a re-bind).
    pub root: Option<isize>,
    pub last_client_size: (i32, i32),
    /// Index of the step that runs next.
    pub step: usize,
}

#[derive(Debug, Clone, Copy)]
struct ChildCache {
    child: SafeHwnd,
    /// Offset of the child's client origin inside the root's client area.
    dx: i32,
    dy: i32,
    at: Instant,
}

struct Runtime {
    cfg: ClickerConfig,
    status: Status,
    root: Option<SafeHwnd>,
    /// Child control under each click step's point.
    children: HashMap<usize, ChildCache>,
    last_client_size: (i32, i32),
    count: u64,
    rounds: u64,
    started: Option<Instant>,
    elapsed_before: Duration,
    /// Monotonic generation: heap entries from an older generation are stale.
    gen: u64,
    last_rebind: Option<Instant>,
    step: usize,
    /// Set by `tick` when the target is minimized and should be auto-hidden; performed
    /// by the scheduler loop *outside* the lock (cross-process window calls can block).
    pending_hide: Option<SafeHwnd>,
    autohide_blocked_until: Option<Instant>,
}

impl Runtime {
    fn new(cfg: ClickerConfig) -> Self {
        let size = first_pick_size(&cfg);
        let root = cfg.target.hwnd.map(SafeHwnd);
        Self {
            cfg,
            status: Status::Idle,
            root,
            children: HashMap::new(),
            last_client_size: size,
            count: 0,
            rounds: 0,
            started: None,
            elapsed_before: Duration::ZERO,
            gen: 0,
            last_rebind: None,
            step: 0,
            pending_hide: None,
            autohide_blocked_until: None,
        }
    }

    fn auto_hides(&self) -> bool {
        self.status.is_active() && self.cfg.hide_when_minimized && self.cfg.mode == ClickMode::Background
    }

    fn elapsed(&self) -> Duration {
        self.elapsed_before + self.started.map(|s| s.elapsed()).unwrap_or_default()
    }
}

/// Client size recorded when the first click point was picked (fallback while minimized).
fn first_pick_size(cfg: &ClickerConfig) -> (i32, i32) {
    cfg.steps
        .iter()
        .find_map(|s| match &s.action {
            Action::Click { point, .. } if point.client_w > 0 => Some((point.client_w, point.client_h)),
            _ => None,
        })
        .unwrap_or((0, 0))
}

#[derive(PartialEq, Eq, PartialOrd, Ord)]
struct Deadline(Instant, u64 /* id */, u64 /* gen */);

struct Inner {
    clickers: HashMap<u64, Runtime>,
    heap: BinaryHeap<Reverse<Deadline>>,
    hires: Option<HighResTimer>,
    notice: String,
    shutdown: bool,
}

pub struct Engine {
    inner: Mutex<Inner>,
    cv: Condvar,
}

impl Engine {
    pub fn new() -> Arc<Self> {
        let engine = Arc::new(Self {
            inner: Mutex::new(Inner {
                clickers: HashMap::new(),
                heap: BinaryHeap::new(),
                hires: None,
                notice: String::new(),
                shutdown: false,
            }),
            cv: Condvar::new(),
        });
        let e = engine.clone();
        std::thread::Builder::new().name("click-scheduler".into()).spawn(move || e.run()).expect("spawn scheduler");
        engine
    }

    // ------------------------------------------------------------------ config sync

    /// Mirror the UI's clicker list into the engine. Running clickers pick up edited
    /// settings on their next tick; clickers no longer in the list are stopped.
    pub fn sync(&self, cfgs: &[ClickerConfig]) {
        let mut g = self.inner.lock().unwrap();
        let ids: Vec<u64> = cfgs.iter().map(|c| c.id).collect();
        g.clickers.retain(|id, _| ids.contains(id));
        for c in cfgs {
            match g.clickers.get_mut(&c.id) {
                Some(rt) => {
                    let target_changed = rt.cfg.target != c.target;
                    let steps_changed = rt.cfg.steps != c.steps || rt.cfg.post_to_root != c.post_to_root;
                    rt.cfg = c.clone();
                    if target_changed {
                        rt.root = c.target.hwnd.map(SafeHwnd).filter(|h| window::is_window(*h));
                        rt.last_client_size = first_pick_size(c);
                        rt.children.clear();
                    }
                    if steps_changed {
                        rt.children.clear();
                        if rt.step >= c.steps.len() {
                            rt.step = 0;
                        }
                    }
                }
                None => {
                    g.clickers.insert(c.id, Runtime::new(c.clone()));
                }
            }
        }
        self.update_hires(&mut g);
        self.cv.notify_all();
    }

    pub fn set_notice(&self, s: String) {
        self.inner.lock().unwrap().notice = s;
    }

    pub fn notice(&self) -> String {
        self.inner.lock().unwrap().notice.clone()
    }

    // ------------------------------------------------------------------ control

    pub fn start(&self, id: u64) {
        let mut g = self.inner.lock().unwrap();
        Self::start_locked(&mut g, id);
        self.update_hires(&mut g);
        self.cv.notify_all();
    }

    fn start_locked(g: &mut Inner, id: u64) {
        let Some(rt) = g.clickers.get_mut(&id) else { return };
        if rt.status.is_active() {
            return;
        }
        if !rt.cfg.target.is_set() {
            rt.status = Status::Error("no target window selected".into());
            return;
        }
        let Some(first) = next_enabled(&rt.cfg.steps, 0) else {
            rt.status = Status::Error("no steps turned on".into());
            return;
        };
        rt.gen += 1;
        rt.count = 0;
        rt.rounds = 0;
        rt.step = first;
        rt.elapsed_before = Duration::ZERO;
        rt.started = Some(Instant::now());
        rt.status = Status::Running;
        rt.children.clear();
        rt.last_rebind = None;
        g.heap.push(Reverse(Deadline(Instant::now(), id, rt.gen)));
    }

    pub fn stop(&self, id: u64) {
        let mut g = self.inner.lock().unwrap();
        let mut roots = Vec::new();
        if let Some(rt) = g.clickers.get_mut(&id) {
            roots.extend(rt.root);
            Self::stop_rt(rt, Status::Idle);
        }
        self.update_hires(&mut g);
        self.cv.notify_all();
        drop(g);
        self.release_unused(roots);
    }

    /// Put windows that were auto-hidden back to minimized once no running clicker
    /// targets them any more.
    fn release_unused(&self, roots: Vec<SafeHwnd>) {
        for root in roots {
            if !ghost::is_auto(root) {
                continue;
            }
            let in_use =
                self.inner.lock().unwrap().clickers.values().any(|r| r.status.is_active() && r.root == Some(root));
            if !in_use {
                ghost::release_auto(root);
            }
        }
    }

    /// Should a minimize of `hwnd` be turned into "hidden but running"?
    pub fn wants_auto_hide(&self, hwnd: isize) -> bool {
        self.inner.lock().unwrap().clickers.values().any(|r| r.auto_hides() && r.root == Some(SafeHwnd(hwnd)))
    }

    fn stop_rt(rt: &mut Runtime, status: Status) {
        if rt.status.is_active() {
            rt.elapsed_before = rt.elapsed();
            rt.started = None;
        }
        rt.gen += 1; // invalidates any heap entry
        rt.status = status;
    }

    pub fn start_all(&self) {
        let mut g = self.inner.lock().unwrap();
        let ids: Vec<u64> = g.clickers.keys().copied().collect();
        for id in ids {
            Self::start_locked(&mut g, id);
        }
        self.update_hires(&mut g);
        self.cv.notify_all();
    }

    pub fn stop_all(&self) {
        let mut g = self.inner.lock().unwrap();
        let mut roots = Vec::new();
        for rt in g.clickers.values_mut() {
            if rt.status.is_active() {
                roots.extend(rt.root);
            }
            Self::stop_rt(rt, Status::Idle);
        }
        g.heap.clear();
        self.update_hires(&mut g);
        self.cv.notify_all();
        drop(g);
        self.release_unused(roots);
    }

    pub fn toggle_all(&self) {
        if self.running_count() > 0 {
            self.stop_all();
        } else {
            self.start_all();
        }
    }

    pub fn toggle(&self, id: u64) {
        let active = self.inner.lock().unwrap().clickers.get(&id).map_or(false, |r| r.status.is_active());
        if active {
            self.stop(id);
        } else {
            self.start(id);
        }
    }

    pub fn running_count(&self) -> usize {
        self.inner.lock().unwrap().clickers.values().filter(|r| r.status.is_active()).count()
    }

    pub fn stats(&self) -> HashMap<u64, ClickerStats> {
        let g = self.inner.lock().unwrap();
        g.clickers
            .iter()
            .map(|(id, rt)| {
                let live = rt.root.filter(|h| window::is_window(*h));
                (
                    *id,
                    ClickerStats {
                        status: rt.status.clone(),
                        count: rt.count,
                        rounds: rt.rounds,
                        elapsed: rt.elapsed(),
                        target_found: live.is_some(),
                        root: live.map(|h| h.0),
                        last_client_size: rt.last_client_size,
                        step: rt.step,
                    },
                )
            })
            .collect()
    }

    /// Run one step right now (UI "Test" button), outside the schedule.
    pub fn test_step(&self, id: u64, step: usize) -> Result<(), String> {
        let mut g = self.inner.lock().unwrap();
        let Some(rt) = g.clickers.get_mut(&id) else { return Err("unknown clicker".into()) };
        if step >= rt.cfg.steps.len() {
            return Err("no such step".into());
        }
        rt.last_rebind = None;
        Self::ensure_target(rt)?;
        Self::fire(rt, step)
    }

    pub fn shutdown(&self) {
        let mut g = self.inner.lock().unwrap();
        g.shutdown = true;
        g.hires = None;
        self.cv.notify_all();
    }

    fn update_hires(&self, g: &mut Inner) {
        let need = g.clickers.values().any(|r| r.status.is_active() && r.cfg.min_wait_ms() < HIRES_THRESHOLD_MS);
        match (need, g.hires.is_some()) {
            (true, false) => g.hires = Some(HighResTimer::acquire()),
            (false, true) => g.hires = None,
            _ => {}
        }
    }

    // ------------------------------------------------------------------ scheduler

    fn run(&self) {
        let mut g = self.inner.lock().unwrap();
        loop {
            if g.shutdown {
                return;
            }
            let now = Instant::now();
            let Some(Reverse(Deadline(when, _, _))) = g.heap.peek() else {
                g = self.cv.wait(g).unwrap();
                continue;
            };
            if *when > now {
                let wait = *when - now;
                g = self.cv.wait_timeout(g, wait).unwrap().0;
                continue;
            }
            let Reverse(Deadline(when, id, gen)) = g.heap.pop().unwrap();
            let Some(rt) = g.clickers.get_mut(&id) else { continue };
            if rt.gen != gen || !rt.status.is_active() {
                continue; // stale
            }
            let mut finished_root = None;
            match Self::tick(rt, when, now) {
                Some(next) => {
                    let gen = rt.gen;
                    g.heap.push(Reverse(Deadline(next, id, gen)));
                }
                None => {
                    let final_status = if rt.status.is_active() { Status::Finished } else { rt.status.clone() };
                    finished_root = rt.root;
                    Self::stop_rt(rt, final_status);
                    self.update_hires(&mut g);
                }
            }
            let pending = g.clickers.get_mut(&id).and_then(|rt| rt.pending_hide.take());
            if pending.is_some() || finished_root.is_some() {
                drop(g);
                if let Some(h) = pending {
                    if let Err(e) = ghost::hide_auto(h) {
                        log::warn!("auto-hide failed: {e}");
                        if let Some(rt) = self.inner.lock().unwrap().clickers.get_mut(&id) {
                            rt.autohide_blocked_until = Some(Instant::now() + AUTOHIDE_RETRY);
                        }
                    }
                }
                if let Some(r) = finished_root {
                    self.release_unused(vec![r]);
                }
                g = self.inner.lock().unwrap();
            }
        }
    }

    /// One scheduled tick. Returns the next deadline, or None when the clicker is done.
    fn tick(rt: &mut Runtime, scheduled: Instant, now: Instant) -> Option<Instant> {
        if rt.cfg.max_seconds > 0 && rt.elapsed() >= Duration::from_secs(rt.cfg.max_seconds) {
            return None;
        }
        if rt.cfg.max_rounds > 0 && rt.rounds >= rt.cfg.max_rounds {
            return None;
        }
        let Some(idx) = next_enabled(&rt.cfg.steps, rt.step) else {
            rt.status = Status::Error("no steps turned on".into());
            return None;
        };
        rt.step = idx;

        if let Err(reason) = Self::ensure_target(rt) {
            rt.status = Status::Waiting(reason);
            return Some(now + REBIND_EVERY);
        }

        if let Some(root) = rt.root {
            let blocked = rt.autohide_blocked_until.map_or(false, |t| now < t);
            if rt.auto_hides() && !blocked && window::is_iconic(root) && !ghost::is_ghosted(root) {
                // Hide first (done by the loop outside the lock), then fire right after.
                rt.pending_hide = Some(root);
                return Some(now + Duration::from_millis(30));
            }
        }

        match Self::fire(rt, idx) {
            Ok(()) => {
                rt.count += 1;
                rt.status = Status::Running;
                let next = next_enabled(&rt.cfg.steps, idx + 1).unwrap_or(idx);
                if next <= idx {
                    rt.rounds += 1; // wrapped around: one full pass done
                }
                rt.step = next;
            }
            Err(e) => {
                rt.status = Status::Waiting(e);
                rt.children.clear();
                rt.root = None;
                return Some(now + REBIND_EVERY);
            }
        }
        if rt.cfg.max_rounds > 0 && rt.rounds >= rt.cfg.max_rounds {
            return None;
        }

        let base = rt.cfg.steps[idx].wait_ms.clamp(1, 3_600_000);
        let wait = if rt.cfg.jitter_ms > 0 {
            let j = rand::thread_rng().gen_range(-(rt.cfg.jitter_ms as i64)..=rt.cfg.jitter_ms as i64);
            (base as i64 + j).max(1) as u64
        } else {
            base
        };
        // Schedule from the planned time to avoid drift, but never pile up a backlog
        // if the machine stalled (sleep, heavy load).
        let mut next = scheduled + Duration::from_millis(wait);
        if next < now {
            next = now + Duration::from_millis(wait.min(5));
        }
        Some(next)
    }

    /// Make sure `rt.root` points at a live window, re-binding by class/process if needed.
    fn ensure_target(rt: &mut Runtime) -> Result<(), String> {
        if let Some(root) = rt.root {
            if window::is_window(root) {
                return Ok(());
            }
            rt.root = None;
            rt.children.clear();
        }
        let now = Instant::now();
        if rt.last_rebind.map_or(false, |t| now - t < REBIND_EVERY) {
            return Err("window not found".into());
        }
        rt.last_rebind = Some(now);
        let t = &rt.cfg.target;
        if t.class.is_empty() && t.process.is_empty() {
            return Err("window closed".into());
        }
        match window::find_matching(&t.class, &t.process, &t.title) {
            Some(h) => {
                rt.root = Some(h);
                rt.children.clear();
                Ok(())
            }
            None => Err(format!("{} not running", if t.process.is_empty() { &t.class } else { &t.process })),
        }
    }

    /// Deliver step `idx` to the resolved target.
    fn fire(rt: &mut Runtime, idx: usize) -> Result<(), String> {
        let root = rt.root.ok_or("no window")?;
        let iconic = window::is_iconic(root);
        if !iconic {
            if let Some(sz) = window::client_size(root) {
                if sz.0 > 0 && sz.1 > 0 {
                    rt.last_client_size = sz;
                }
            }
        }
        let to_root = rt.cfg.post_to_root || rt.cfg.target.is_chromium();
        let action = rt.cfg.steps[idx].action.clone();
        match action {
            Action::Click { point, button, kind } => {
                let (w, h) = rt.last_client_size;
                let (mut x, mut y) = point.resolve(w, h);
                if rt.cfg.jitter_px > 0 {
                    let mut rng = rand::thread_rng();
                    x += rng.gen_range(-rt.cfg.jitter_px..=rt.cfg.jitter_px);
                    y += rng.gen_range(-rt.cfg.jitter_px..=rt.cfg.jitter_px);
                }
                match rt.cfg.mode {
                    ClickMode::Foreground => {
                        if iconic {
                            return Err("window minimized (foreground mode needs it visible)".into());
                        }
                        let (sx, sy) = window::client_to_screen(root, x, y).ok_or("cannot map to screen")?;
                        if input::send_input_click(sx, sy, button, kind) {
                            Ok(())
                        } else {
                            Err("SendInput rejected".into())
                        }
                    }
                    ClickMode::Background => {
                        let (target, tx, ty) =
                            if to_root { (root, x, y) } else { Self::child_target(rt, idx, root, x, y, iconic) };
                        if input::post_click(target, tx, ty, button, kind) {
                            Ok(())
                        } else {
                            Err(post_failed())
                        }
                    }
                }
            }
            Action::Key { vk } => {
                let target = Self::key_target(root, to_root);
                if input::post_key(target, vk) {
                    Ok(())
                } else {
                    Err(post_failed())
                }
            }
            Action::Text { text } => {
                let target = Self::key_target(root, to_root);
                if input::post_text(target, &text) {
                    Ok(())
                } else {
                    Err(post_failed())
                }
            }
        }
    }

    /// Keyboard input goes to the control that owns focus inside the target (the edit box
    /// a user would type into), falling back to the window itself.
    fn key_target(root: SafeHwnd, to_root: bool) -> SafeHwnd {
        if to_root {
            return root;
        }
        window::focused_child(root).unwrap_or(root)
    }

    /// Resolve (and cache) the deepest child control under a click step's point. While the
    /// window is minimized `ChildWindowFromPointEx` is unreliable, so we keep using the
    /// cached child and its offset, which do not change while minimized.
    fn child_target(rt: &mut Runtime, idx: usize, root: SafeHwnd, x: i32, y: i32, iconic: bool) -> (SafeHwnd, i32, i32) {
        let now = Instant::now();
        let fresh = rt
            .children
            .get(&idx)
            .copied()
            .filter(|c| window::is_window(c.child) && (iconic || now - c.at < CHILD_CACHE_TTL));
        let cache = match fresh {
            Some(c) => c,
            None if iconic => return (root, x, y),
            None => {
                let (child, _, _) = window::deepest_child_at(root, x, y);
                let (dx, dy) = if child == root { (0, 0) } else { window::child_offset(root, child) };
                let c = ChildCache { child, dx, dy, at: now };
                rt.children.insert(idx, c);
                c
            }
        };
        (cache.child, x - cache.dx, y - cache.dy)
    }
}

/// First switched-on step at or after `from`, wrapping around.
fn next_enabled(steps: &[crate::config::Step], from: usize) -> Option<usize> {
    let n = steps.len();
    (0..n).map(|k| (from + k) % n).find(|&i| steps[i].enabled)
}

fn post_failed() -> String {
    "PostMessage failed (elevated target? run ArkoClick as administrator)".into()
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.shutdown();
    }
}
