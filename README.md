# CATA Pharmacy Duty Panel

A Windows desktop app for managing pharmacy duty rosters and sending the current day's pharmacy names to compatible iPIXEL Bluetooth LED matrix panels, including the CATA CT-4568 (96 × 16).

The app is an independent community project. It is not affiliated with CATA, iPIXEL, or the authors of the libraries and protocol documentation listed in [Third-party notices](THIRD_PARTY_NOTICES.md).

## Download

Download **NobetPanel.exe** from the latest [GitHub Release](https://github.com/TechnoQuantum/cata-nobet-panel/releases). It is a single-file Windows executable; Python does not need to be installed. Windows SmartScreen may show a warning because the executable is not code-signed.

## Features

- Monthly calendar and roster table, with import from `.xlsx` and `.zip` files
- Manual and bulk entry of duty dates and pharmacy names
- Animated preview and controls for text/background color, effect, speed, brightness, font, orientation, and mirroring
- Manual send for today's or a selected roster entry, plus free-text and image/GIF sending
- Optional save to a panel slot and advanced iPIXEL commands
- Optional Windows sign-in startup, minimized to the taskbar
- Automatic BLE scanning and daily send when the panel is found or reconnects
- Optional red pharmacy E logo with a white rim and blinking red frame, followed by a scrolling duty ticker
- Logo ticker playback is slowed by 25%; the app keeps a saved copy and refreshes the live slot every 10 seconds to avoid the panel's boot-screen fallback
- Optional “we are on duty today” scrolling message when your pharmacy name matches that roster date

The display text omits addresses by default. The app scans for compatible BLE names with the `LED_BLE_` prefix instead of assuming one device address.

An anonymized date/name sample that can be loaded through the CSV/text import is available at [`examples/anonymized_roster.csv`](examples/anonymized_roster.csv). Every pharmacy name in the sample is replaced with generic placeholders; it contains no real pharmacy assignments.

The E logo is optional for regular roster sends; enable it with **Kayan nöbet listesinde E logosu göster (isteğe bağlı)**. The logo stays fixed while the ticker scrolls in a separate clipped area beside it. To enable the on-duty message, enter your pharmacy name in **Bizim eczane adımız** and enable the option below it. When that name matches the current duty date, the ticker shows `<date> - BUGÜN NÖBETÇİYİZ: <pharmacy>` with the fixed E logo. Otherwise it shows the regular roster, with the logo only when the optional setting is enabled.

The app window and Windows executable use the custom icon in [`assets/nobet_panel.ico`](assets/nobet_panel.ico); the matching PNG is included for documentation and repackaging.

## Requirements

- Windows 10/11 with a Bluetooth LE adapter
- A compatible iPIXEL panel powered on and in Bluetooth range
- Close/disconnect the phone app while connecting: the panel may only allow one BLE client at a time
- For automatic sending, Windows must be awake and the app running. The startup option launches the app minimized and continues polling for the panel.

## Build from source

Python 3.12 is recommended. In PowerShell:

```powershell
py -3.12 -m venv .venv
.\.venv\Scripts\Activate.ps1
python -m pip install -r requirements.txt
python -m pip install -r requirements-build.txt
./build_windows.ps1
```

The resulting single-file executable is written to `dist/NobetPanel.exe`.

The core source is `nobet_panel.py`. Runtime settings and imported rosters are stored per Windows user in `%LOCALAPPDATA%\NobetPanel\nobet_listesi.json`; this file is not part of this repository or the release asset.

## BLE protocol and upstream projects

This app delegates device commands and image/text encoding to [`pypixelcolor`](https://github.com/lucagoc/pypixelcolor) (MIT). The iPIXEL BLE service/command details are based on the [iPIXEL protocol documentation](https://github.com/cagcoach/ha-ipixel-color/blob/main/iPIXEL-Protocol-Documentation.md). BLE discovery uses [`Bleak`](https://github.com/hbldh/bleak); image handling uses [`Pillow`](https://github.com/python-pillow/Pillow). Versions are pinned or constrained in the requirements files. See [Third-party notices](THIRD_PARTY_NOTICES.md) for license information.

The generic on-duty message option was inspired by the on-duty display concept shown on the [REGO Nöbet product page](https://regosoft.com.tr/urun/rego-nobet-akilli-nobetci-eczane-tabelasi). No REGO graphics or product assets are included.

## Limitations

- Panel behavior varies by firmware. Slot enumeration/readback can be unavailable through the library/device API; the app's slot list may therefore reflect its local send manifest rather than a complete device readback.
- Automatic sending is best-effort. The PC needs to be awake, Bluetooth enabled, and able to connect to the panel.
- The small 96 × 16 panel limits how much of a logo and message is visible at once; the ticker scrolls the full message over time.
- This software has not been validated for emergency alerting or other safety-critical use. Confirm the displayed duty roster against the official source.

## License

No license is granted for this application's original source files. Third-party components retain their own licenses; see [Third-party notices](THIRD_PARTY_NOTICES.md).
