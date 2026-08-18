# Windows Audio Switcher

Tauri 2 tray app (Rust backend only). Switches every Windows audio default role
(Console / Multimedia / Communications) at once. There is no frontend:
`dist/index.html` is an empty stub that only exists to satisfy Tauri config - no
window is ever created, and none should be added.

## Commands (run from `src-tauri/`)
- Build / lint: `cargo build`, `cargo clippy`
- Tests: `cargo test` - CAUTION: the audio tests switch the REAL default audio
  endpoint (they restore it, but don't run them on a machine with active audio).
- Release bundle: from repo root, `bun run tauri build` (needs signing keys for
  updater artifacts), or `scripts/build.ps1`, which reads the key from
  `~/.tauri/audioswitch.key`.

## Windows-only, but must stay cross-compilable
All platform code is `#[cfg(target_os = "windows")]` with stubs for other
platforms (`main.rs`, `lib.rs`). Keep the stubs compiling - `winreg` is a
`[target.'cfg(windows)'.dependencies]` dep for exactly this reason.

## Diagnostics (release has no console)
`log_line()` appends to `%LOCALAPPDATA%\com.audioswitch.app\app.log` (rotates at
256 KiB to `app.log.old`). `settings.txt` in the same dir (`"true"`/`"false"`)
controls update-check-on-startup. This is the only way to observe the app in
release.

## Audio core (`src-tauri/src/audio/`)
ALL Core Audio work (list/switch/notifications) runs on one dedicated STA thread
with a Win32 message pump (`watcher.rs`); MMDevApi crashes if apartments are
mixed. Never call Core Audio directly from another thread - use
`audio::list_devices` / `audio::set_default`, which dispatch jobs to that thread.

## Tray menu rules (hard-won)
- Rebuild the menu ONLY from menu events (Refresh, device switch, toggles).
  `tray.set_menu` while a popup is open dismisses/flashes it - so
  `rebuild_tray_menu_when_closed` defers rebuilds while one of our `#32768`
  popups is open.
- Escape `&` in device labels to `&&` (Win32 menu mnemonic).
- The popup opens with its bottom inside the taskbar; `spawn_menu_work_area_clamp`
  lifts it into the work area via an `EVENT_SYSTEM_MENUPOPUPSTART` WinEvent hook.
  Tray click events arrive only AFTER the popup closes, so a click handler cannot
  do this.
- Multi-monitor tray support is out of scope: the tray (notification area) only
  shows on the primary monitor's taskbar, so the clamp's primary-monitor work
  area is fine - don't consider per-monitor handling.

## Autostart ordering
Winlogon launches `HKCU\...\Run` values in INSERTION order (not name order), so a
re-enabled entry always starts last. `reorder_autostart_to_front` moves it to the
front inside a TxR transaction (atomic vs power loss), on startup and after
enabling. Keep `winreg` at 0.10 - it must unify with auto-launch's copy. Enabling
goes through tauri-plugin-autostart (it also maintains `StartupApproved\Run` for
Task Manager).

## Releases
- `Cargo.toml` version is the source of truth; CI rewrites it from the git tag
  (`v1.0.6` -> `1.0.6`). No manual bump.
- GitHub sanitizes spaces in asset names (spaces -> dots): release asset names
  and updater manifest URLs must use hyphens or every update 404s.
- Tags containing a hyphen become GitHub pre-releases and never reach stable
  installs via `/releases/latest` (by design).
- Signing: `TAURI_SIGNING_PRIVATE_KEY` + password (CI secrets; the local build
  script reads `~/.tauri/audioswitch.key`).
