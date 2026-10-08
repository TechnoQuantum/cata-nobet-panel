$ErrorActionPreference = "Stop"

python -m PyInstaller `
  --noconfirm `
  --clean `
  --onefile `
  --windowed `
  --name NobetPanel `
  --distpath dist `
  --workpath build `
  --collect-all pypixelcolor `
  --collect-all bleak `
  --collect-all winrt `
  --collect-all PIL `
  nobet_panel.py

