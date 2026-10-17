//! ArkoClick library crate: everything except the executable entry point lives here so
//! integration tests can drive the engine against real windows.

pub mod config;
pub mod engine;
pub mod ui;
pub mod update;
pub mod win;
