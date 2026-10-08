# Third-party notices

This application uses and/or redistributes the following third-party software. Their copyrights and licenses remain with their respective authors. The source links below identify upstream projects; this application is an independent project and is not endorsed by them.

| Component | Use | Upstream | License |
|---|---|---|---|
| pypixelcolor 0.5.0 | iPIXEL command encoding and BLE panel control | [lucagoc/pypixelcolor](https://github.com/lucagoc/pypixelcolor) | MIT; full license text is in [`licenses/pypixelcolor-MIT.txt`](licenses/pypixelcolor-MIT.txt) |
| iPIXEL protocol documentation | BLE protocol reference | [cagcoach/ha-ipixel-color](https://github.com/cagcoach/ha-ipixel-color) | Refer to the upstream repository's license and documentation terms |
| Bleak 3.0.2 | Bluetooth Low Energy scanning and transport | [hbldh/Bleak](https://github.com/hbldh/bleak) | MIT |
| Pillow 12.3.0 | Image/GIF handling | [python-pillow/Pillow](https://github.com/python-pillow/Pillow) | MIT-CMU |
| PyWinRT 3.2.1 | Windows Runtime Bluetooth bindings used by Bleak | [pywinrt/pywinrt](https://github.com/pywinrt/pywinrt) | MIT |
| crccheck | CRC support required by pypixelcolor | [MightyPork/crccheck](https://github.com/MightyPork/crccheck) | MIT |

The Windows release is built with [PyInstaller](https://github.com/pyinstaller/pyinstaller). The runtime dependency versions used for the published build are recorded in `requirements.txt`; the PyInstaller build dependency is recorded in `requirements-build.txt`.

## Attribution

The panel protocol implementation is provided by the upstream `pypixelcolor` project. The application does not copy or vendor its source code. Protocol-specific behavior is informed by the `ha-ipixel-color` documentation linked above. Please consult upstream repositories for full notices and license terms.

