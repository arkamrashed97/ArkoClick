//! Scoped multimedia-timer resolution. Windows sleeps/waits tick at ~15.6 ms by default;
//! `timeBeginPeriod(1)` brings that to ~1 ms. It costs some power, so we only hold it
//! while a clicker with a short interval is running.

use windows::Win32::Media::{timeBeginPeriod, timeEndPeriod};

pub struct HighResTimer;

impl HighResTimer {
    pub fn acquire() -> Self {
        unsafe {
            timeBeginPeriod(1);
        }
        Self
    }
}

impl Drop for HighResTimer {
    fn drop(&mut self) {
        unsafe {
            timeEndPeriod(1);
        }
    }
}
