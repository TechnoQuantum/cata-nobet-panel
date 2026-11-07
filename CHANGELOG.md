# Changelog

## 2.0.0 - 2026-11-07

Complete rewrite in Rust with a Slint UI. The 1.x Python app is kept in `legacy/`.

### Fixed

- Panel fell back to its default "LED" screen after a reboot or power cut: content was played from the volatile live slot and the saved slot was never selected. Content is now stored in a program slot that is selected as the panel's startup program.
- Sends were reported successful without the panel confirming them. Every window and command now waits for the panel's acknowledgement (late acknowledgements of earlier commands are ignored), and the final "transfer complete" status is required.
- Intermittent communication failures: writes used fixed 244-byte chunks regardless of the negotiated MTU, notifications were re-subscribed for every request, and a stale acknowledgement could satisfy the wait for the next window.
- Button presses were silently ignored while the 5-second background scan ran; failed automatic sends retried every 5 seconds. Jobs are now queued; automatic retries back off from 30 s to 10 min.
- Changing the logo or display settings did not update the panel; it is now re-sent automatically when its content changes.
- Logo GIFs were re-encoded with a 256-colour palette per frame, making transfers large and slow. They now use one small global palette and stay under a size budget (≈37 KB).
- The "E" logo sat one row too low inside its frame.
- Own-pharmacy preview and the actual send disagreed; Turkish upper-casing turned "i" into "I" instead of "İ"; a corrupt data file was overwritten on the next save; more than 255 characters crashed the text encoder.
- Two copies of the app could run at once and fight over the single Bluetooth connection.
- The panel sometimes acknowledged every window of an upload as "complete" without storing anything, then showed its default "LED" screen. Such uploads are now detected; the slot is cleared and the content uploaded again.

### Added

- Windows service (`NobetPanel`) that updates the panel even when nobody is signed in, plus an installer (`Kur.cmd` / `install.ps1`) and `uninstall.ps1`. Service and window share `C:\ProgramData\NobetPanel` and coordinate panel access.
- Confirmation dialogs with preview before sending, deleting, overwriting imports, clearing panel memory and powering off.
- Panel size detection, connection diagnostics, device info, log page and rotating log file.
- `--send-today [--quiet]` command-line mode.
- The saved program slot is re-selected every 5 minutes (`keepalive_min`), so the list returns soon after an unnoticed panel restart.
- Single-file setup `NobetPanel-<version>-Setup.exe` next to the zip package.
- `--command info|show-slot N|delete-slot N|clear|raw HEX` for diagnostics.

## 1.0.4 - 2026-10-09

- During 08:30–18:00 gaps, select today's upcoming roster and display its exact duty window so the prior shift is not mistaken for the current one.

## 1.0.3 - 2026-10-08

- Keep the E logo fixed beside a continuously looping duty ticker; upload the animation once to the live display slot instead of retransmitting it every 10 seconds.
- Save compact scrolling text as the selected-slot fallback, reducing BLE transfer failures during logo sends.
- Debounce automatic reconnects: a single missed BLE scan no longer restarts the animation; two consecutive misses confirm the panel is offline.
- Reconnect using the discovered BLE address and retry interrupted transfers up to three times.
- Select roster dates by duty windows: regular shifts start at 18:00 and end at 08:30; Sunday shifts run Sunday 08:30 through Monday 08:30.

## 1.0.2 - 2026-10-08

- Display logo tickers through live slot 0 to prevent the panel from returning to its default boot screen after selecting a saved slot.
- Keep a saved copy in the selected slot and refresh the live animation every 10 seconds while the app is connected.

## 1.0.1 - 2026-10-08

- Added a custom pharmacy E application and executable icon.
- Added a blinking red frame and white-rimmed red E logo to animated roster sends, with a toggle in the app.
- Added the date to the optional “BUGÜN NÖBETÇİYİZ” message and included the logo in its ticker.
- Improved the roster preview to show the logo alongside its scrolling message.
- Slowed logo-ticker playback by 25% while preserving the speed control, and replay the saved panel slot to prevent the animation from stopping after one pass.

## 1.0.0 - 2026-10-08

- Initial Windows release for iPIXEL-compatible BLE LED matrix panels.
- Added roster import, calendar/list views, animation preview, display settings, and automatic daily reconnect sending.
- Added minimized Windows startup and advanced panel controls.
- Added an optional pharmacy-name match that shows an animated “BUGÜN NÖBETÇİYİZ” message on that pharmacy's duty day.
- Added an anonymized sample roster and third-party attribution notes.
