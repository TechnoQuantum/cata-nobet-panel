# Changelog

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
