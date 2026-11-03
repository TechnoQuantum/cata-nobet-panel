# CATA Pharmacy Duty Panel

A Windows app that keeps a pharmacy's duty roster on a Bluetooth LED matrix panel (iPIXEL-compatible, e.g. CATA CT-4568, 96 × 16). It imports the monthly roster, picks the pharmacies on duty for the current shift and sends them to the panel — automatically, every day, including after reboots and power cuts.

Turkish instructions: [README_TR.md](README_TR.md).

The app is an independent community project, not affiliated with CATA, iPIXEL or the projects listed in [Third-party notices](THIRD_PARTY_NOTICES.md).

[![Made with Slint](https://raw.githubusercontent.com/slint-ui/slint/master/logo/MadeWithSlint-logo-light.svg)](https://slint.dev)

## Install

1. Download `NobetPanel-<version>.zip` from the [Releases](https://github.com/TechnoQuantum/cata-nobet-panel/releases) page and extract it.
2. Double-click **Kur.cmd** and accept the administrator prompt.

The installer:

- copies the app to `C:\Program Files\NobetPanel`,
- moves existing data from `%LOCALAPPDATA%\NobetPanel` (1.x) to `C:\ProgramData\NobetPanel`,
- installs the **NobetPanel** Windows service, which starts with Windows (delayed start) and updates the panel even when nobody is signed in; it restarts itself if it stops unexpectedly,
- starts the window minimized at sign-in and adds Start-menu and desktop shortcuts,
- removes the 1.x sign-in entry.

Uninstall with `uninstall.ps1` in the install folder (`-RemoveData` also deletes the roster and settings).

The executable is not code-signed, so SmartScreen may warn on first start.

## How it works

- **Service and window.** The service does the automatic sending. The window is for editing the roster, changing the look and manual actions. Both use the same data file; a machine-wide lock ensures only one of them talks to the panel at a time, and each picks up the other's changes within seconds.
- **Survives reboots.** Content is stored in a panel program slot (default 1) and that slot is selected as the panel's startup program. 1.x played the logo ticker from the volatile live slot, so a panel reboot or power cut left it on its default "LED" screen.
- **Delivery confirmation.** Every 12 KB window and every command waits for the panel's acknowledgement; a send is only reported successful once the panel confirms it. Failed sends retry three times, automatic retries back off (30 s → 10 min).
- **Re-sends when something changes.** A fingerprint of what the panel shows is stored; when the roster entry or any display setting (logo, colours, speed, effect, font, mirroring, brightness) changes, the panel is updated automatically.
- **Shift-aware.** Regular shifts run 18:00–08:30, Sunday duty Sunday 08:30–Monday 08:30; the duty date switches at 08:30.
- **Confirmations.** Sends show a preview first (optional), and deleting records, overwriting imports, clearing panel memory or powering it off ask before acting.

## Features

- Monthly calendar, roster import from `.xlsx`/`.zip` (chamber monthly lists), CSV/text bulk import with a change summary, CSV export
- Live LED preview that matches what the panel will show
- Text and background colour, effect, speed, brightness, custom TTF/OTF font (built-in Unifont covers Turkish)
- Optional fixed pharmacy "E" logo with a blinking frame beside the scrolling list
- Optional "BUGÜN NÖBETÇİYİZ" message when your own pharmacy is on duty
- Orientation and horizontal/vertical mirroring for upside-down mounting
- Free text and image/GIF sending, panel slot management, device info, time sync, raw commands
- Log page and log file (`C:\ProgramData\NobetPanel\nobet_panel.log`)
- Command line: `NobetPanel.exe --send-today [--quiet]` sends today's duty once and writes `last_send.json`

## Requirements

- Windows 10/11 with Bluetooth LE
- The panel powered on and in range. A weak signal (below about −80 dBm) causes intermittent failures; move the PC closer or use a USB Bluetooth adapter.
- Close the phone app: the panel accepts only one Bluetooth connection.

## Build from source

Requires Rust (stable). With the MSVC toolchain just run `./build_windows.ps1`; with the GNU toolchain pass a MinGW-w64 `bin` folder (dlltool, as, windres):

```powershell
./build_windows.ps1 -MinGW C:\path\to\mingw64\bin
```

This runs the tests and writes `dist\NobetPanel.exe` and the installable `dist\NobetPanel-<version>.zip`.

Source layout: `src/protocol.rs` (iPIXEL BLE protocol), `src/ble.rs` (WinRT Bluetooth), `src/render.rs` (text/logo/GIF rendering), `src/worker.rs` (connection, sending, automatic mode), `src/service.rs` (Windows service), `src/store.rs` (data file), `ui/` (Slint UI). The 1.x Python app is kept in `legacy/`.

For testing without touching the installed copy, set `NOBETPANEL_DATA_DIR` to a separate folder; the app then also uses a separate single-instance lock and never changes the sign-in entry.

## Protocol

The BLE protocol implementation is written from the [iPIXEL protocol documentation](https://github.com/cagcoach/ha-ipixel-color/blob/main/iPIXEL-Protocol-Documentation.md) and verified byte-for-byte against [`pypixelcolor`](https://github.com/lucagoc/pypixelcolor) 0.5.0 output in the unit tests.

The on-duty message option was inspired by the display concept on the [REGO Nöbet product page](https://regosoft.com.tr/urun/rego-nobet-akilli-nobetci-eczane-tabelasi). No REGO assets are included.

## Limitations

- Panel behaviour varies by firmware. The slot list reflects what this app sent, not a readback from the panel.
- The PC must be on with Bluetooth enabled for automatic updates.
- Not validated for safety-critical use; check the displayed roster against the official source.

## License

No license is granted for this application's original source files. Third-party components keep their own licenses; see [Third-party notices](THIRD_PARTY_NOTICES.md).
