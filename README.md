# ArkoClick

**Developer:** Arkam Rashed  
**Email:** arko313m@gmail.com  
**Phone / WhatsApp:** +201008446123

Multi-target background auto clicker for Windows, written in Rust (egui + Win32). Version 0.2.0.

## Made for everyone

- **Three tabs.** *Simple clicker*: choose a spot, choose a speed, press Start. *Advanced
  clicker*: sequences of clicks, keys and text with every option. *Keyboard*: press a key or
  type a text into a window, repeatedly.
- **Step-by-step cards** (1, 2, 3, 4) with big buttons, speed presets in plain words
  ("Normal · 5/sec"), and a zoomed picture of the exact spot that will be clicked.
- **English and Arabic.** The Arabic interface is fully translated and mirrored
  right-to-left, with correctly joined letters (shaping via `ar-reshaper`, ordering via
  `unicode-bidi`). Text is typed into a native Windows box, so Arabic input is perfect.
- **Day, night, or follow Windows**, switchable from the header in one click.

## What it does

- **Clicks inside a chosen window, not at a screen position.** Each clicker is bound to a
  window (by handle at runtime, by class + process name across restarts) and points inside
  its client area. Input is delivered with `PostMessage`, so the real mouse never moves and
  you keep using the PC.
- **Step sequences.** A clicker runs a list of steps in order and repeats: mouse clicks
  (left/right/middle, single/double, each with its own point), key presses, and typed text
  (any language, Arabic and emoji included). Every step has its own wait time.
- **Keeps working when the target is minimized.** While a clicker runs, minimizing its
  window turns into "hidden but running" (invisible, click-through, not minimized), because
  many apps, all Chromium browsers included, ignore input while minimized. Clicking the
  window's taskbar button brings it back; stopping the clicker returns it to minimized. A
  manual **Hide window (keep running)** button does the same on demand.
- **Survives resize / maximize.** Points are stored relative to the client size.
- **Many clickers at once**, on different windows or the same one, all driven by **one
  scheduler thread** (a min-heap of deadlines on a condvar). Idle CPU is zero.
- **Profiles**: named sets of clickers (New / Copy / Rename / Delete) to switch between.
- **Hotkeys**: global start/stop-all and emergency stop, plus an optional hotkey per
  clicker. Shortcuts are recorded by pressing them (Ctrl / Alt / Shift supported).
- **Launch with Windows** (HKCU Run key), start hidden in the tray, tray menu (start/stop
  all, show hidden windows, quit), auto-run selected clickers at launch.
- **Picker overlay** with magnifier and crosshair, or choose a window from a list.
- Segoe UI text with adjustable size; Segoe UI Symbol for icons.
- Config lives in `%APPDATA%\ArkoClick\config.json` (v0.1 configs are migrated).

## Measured in Chrome

`cargo run --release --example chrome_probe` with the test page open, 40 actions each:

| Window state | Clicks | Keys |
|---|---|---|
| Visible | 40 / 40 | 40 / 40 |
| Minimized (raw, helper off) | 0 / 40 | 0 / 40 |
| Hidden with ArkoClick | 40 / 40 | 40 / 40 |
| Minimized in the middle of a run (helper on) | 40 / 40 | |

Text steps typed 95 of 95 characters (Arabic included) both visible and hidden.

## Known limits

- Apps that read **raw input / DirectInput** (most games) ignore posted messages. Use
  Foreground mode for those (it moves the real cursor and needs the window visible).
- Clicking into a window that runs **as administrator** requires ArkoClick to run as
  administrator too (UIPI).
- **Key steps** produce the character of the target's active keyboard layout (A types "ش"
  on an Arabic layout). Use a **Text step** for exact text. Modifier combos (Ctrl+C) cannot
  be faked through messages.
- Key and text steps go to the field that has focus inside the target. Click the field once,
  or put a Click step on it before the Key/Text step.
- egui draws Arabic unjoined inside ArkoClick's own text boxes; what gets typed is correct.

## Build

Rust stable with the `x86_64-pc-windows-gnu` host (or MSVC with Visual Studio Build Tools).
The GNU toolchain also needs MinGW-w64 binutils (`dlltool`, `as`) because the `windows`
crate links through `raw-dylib`.

```
cargo build --release
cargo test
```

For the GNU toolchain, copy `.cargo/config.toml.example` to `.cargo/config.toml`, point
`linker` and `dlltool` at your MinGW-w64 install, and put its `bin` folder on `PATH` so
`windres` embeds the icon and version info. The example also moves the target directory,
because `dlltool` fails on paths that contain spaces.

### Installer

Needs [Inno Setup 6](https://jrsoftware.org/isinfo.php):

```
ISCC.exe installer\ArkoClick.iss
```

Builds `dist/ArkoClick-Setup-<version>.exe` from `dist/ArkoClick.exe` (copy the release build
there first). Per-user install to `%LOCALAPPDATA%\Programs\ArkoClick` (no administrator
rights), English or Arabic wizard, Start-menu entries (app, test page, uninstall), optional
desktop shortcut and "start with Windows", and a short guide shown at the end
(`installer/guide-*.txt`). Uninstall removes everything, including the startup entry. The
exe and installer are not code-signed, so Windows SmartScreen warns on first run.

`dist/` holds a ready-to-run `ArkoClick.exe` (about 4.3 MB, no runtime needed) and the test
page `ArkoClick-TestTarget.html` (source in `test-target/`). The page counts clicks, key
presses and typed characters, and mirrors the counters into its window title.

Tests create real Win32 windows and verify: clicks reach the right child control at the right
coordinates, delivery while minimized and while hidden, minimize-to-hidden and back on stop,
step order, key and Unicode text delivery, action limits, re-binding after the window is
recreated, and v0.1 config migration.

## Layout

```
src/
  main.rs          entry, single instance, flags (--autostart / --hidden), crash safety
  config.rs        data model (profiles, clickers, steps, hotkeys) + JSON persistence
  engine/mod.rs    scheduler thread, target binding, step dispatch, auto-hide
  win/             Win32 wrappers
    window.rs      lookup, geometry, child-control hit testing, focus
    input.rs       PostMessage clicks / keys / text, SendInput
    ghost.rs       hide a window without minimizing it (+ crash recovery)
    watch.rs       WinEvent hook: minimize -> hidden, activation -> visible
    hotkey.rs      global hotkeys thread
    tray.rs        tray icon and menu
    picker.rs      full-screen target picker (GDI)
    autostart.rs, screen.rs, timer.rs
    textdlg.rs     native text-entry box (Arabic-correct)
  ui/
    mod.rs         window, header, tabs, task list, shared editor parts
    simple.rs      Simple clicker tab
    advanced.rs    Advanced clicker tab
    keyboard.rs    Keyboard tab
    settings.rs    Settings window
    widgets.rs     language-aware rows, cards, chips, switches
    rtl.rs         Arabic shaping + bidi for display
    theme.rs       fonts, light/dark colours
tests/background_click.rs   end-to-end Win32 tests
examples/chrome_probe.rs    Chromium delivery measurement
examples/picker_demo.rs     opens the picker overlay (Arabic hint) for visual checks
test-target/                click/key/text counter page
```

## Credits

Picker and tray approach adapted from
[xiaobainiuniu/auto-clicker](https://github.com/xiaobainiuniu/auto-clicker) (MIT).
Background-click technique informed by
[PriyanshuGeTRekT/WinClicker](https://github.com/PriyanshuGeTRekT/WinClicker) (C#, MIT) and
[Cling2223/auto-clicker](https://github.com/Cling2223/auto-clicker).
