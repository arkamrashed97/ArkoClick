//! ArkoClick: multi-target background auto clicker for Windows.
//!
//! Flags: `--autostart` (set by the Run registry entry) starts hidden in the tray;
//! `--hidden` does the same manually.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use arkoclick::config::{AppConfig, APP_TITLE};
use arkoclick::{engine, ui, win};
use std::sync::Arc;
use windows::Win32::Foundation::ERROR_ALREADY_EXISTS;
use windows::Win32::System::Threading::CreateMutexW;

/// Only one instance: a second launch just brings the first window to the front.
fn claim_single_instance() -> bool {
    let name = win::to_wide("Local\\ArkoClick.SingleInstance");
    unsafe {
        let _ = CreateMutexW(None, false, windows::core::PCWSTR(name.as_ptr()));
        windows::Win32::Foundation::GetLastError() != ERROR_ALREADY_EXISTS
    }
}

fn main() -> eframe::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    if !claim_single_instance() {
        if let Some(h) = win::window::find_by_title(APP_TITLE) {
            win::window::show_and_focus(h);
        }
        return Ok(());
    }

    let args: Vec<String> = std::env::args().skip(1).collect();
    let launched_by_autostart = args.iter().any(|a| a == "--autostart");
    let mut cfg = AppConfig::load();
    // Keep the checkbox truthful if the user edited the registry by hand.
    cfg.autostart = win::autostart::is_enabled();
    let start_hidden = launched_by_autostart || cfg.start_minimized || args.iter().any(|a| a == "--hidden");

    // Windows a crashed previous run left invisible get restored first.
    win::ghost::recover_after_crash();
    // ...and if this run panics, never leave hidden windows behind.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        win::ghost::show_all();
        default_hook(info);
    }));

    let engine = engine::Engine::new();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(APP_TITLE)
            .with_inner_size([960.0, 720.0])
            .with_min_inner_size([760.0, 520.0])
            .with_icon(Arc::new(egui::IconData { rgba: ui::theme::LOGO_64.to_vec(), width: 64, height: 64 }))
            .with_visible(!start_hidden),
        vsync: true,
        ..Default::default()
    };

    let bindings = win::hotkey::Bindings {
        toggle_all: cfg.hotkey_toggle_all,
        stop_all: cfg.hotkey_stop_all,
        clickers: cfg.clickers().iter().filter(|c| c.hotkey.is_set()).map(|c| (c.id, c.hotkey, c.name.clone())).collect(),
    };
    let engine_for_app = Arc::clone(&engine);

    eframe::run_native(
        APP_TITLE,
        options,
        Box::new(move |cc| {
            // Background threads get an egui Context so they can wake the UI after acting.
            let ctx_hk = cc.egui_ctx.clone();
            win::hotkey::spawn(bindings, Arc::clone(&engine), move || ctx_hk.request_repaint());
            let ctx_tray = cc.egui_ctx.clone();
            win::tray::spawn(Arc::clone(&engine), move || ctx_tray.request_repaint());
            let ctx_watch = cc.egui_ctx.clone();
            win::watch::spawn(Arc::clone(&engine), move || ctx_watch.request_repaint());
            Ok(Box::new(ui::App::new(cc, cfg, engine_for_app, start_hidden)))
        }),
    )
}
