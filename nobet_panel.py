from __future__ import annotations

import asyncio
import calendar
import csv
import io
import json
import math
import os
import queue
import re
import sys
import tempfile
import threading
import tkinter as tk
import tkinter.font as tkfont
import unicodedata
import zipfile
import xml.etree.ElementTree as ET
from datetime import date, datetime, timedelta
from pathlib import Path
from tkinter import colorchooser, filedialog, messagebox, ttk

APP_TITLE = "Nöbetçi Eczane Paneli"
DEVICE_PREFIX = "LED_BLE_"
ANIMATIONS = {
    "Sabit": 0,
    "Sola kaydır": 1,
    "Sağa kaydır": 2,
    "Yanıp sön": 5,
    "Nefes efekti": 6,
    "Kar efekti": 7,
}

LOCAL_APP_DATA = Path(os.environ.get("LOCALAPPDATA", Path.home())) / "NobetPanel"
LOCAL_APP_DATA.mkdir(parents=True, exist_ok=True)
DATA_PATH = LOCAL_APP_DATA / "nobet_listesi.json"


def parse_date(value: str) -> date:
    value = value.strip()
    for fmt in ("%Y-%m-%d", "%d.%m.%Y", "%d/%m/%Y", "%d-%m-%Y"):
        try:
            return datetime.strptime(value, fmt).date()
        except ValueError:
            pass
    raise ValueError(f"Tarih okunamadı: {value}")


def canonical_record(raw: dict) -> dict:
    d = parse_date(str(raw.get("date", "")))
    name = str(raw.get("pharmacy", "")).strip()
    if not name:
        raise ValueError("Eczane adı zorunludur.")
    return {"date": d.isoformat(), "pharmacy": name}


def format_display(record: dict) -> str:
    d = parse_date(record["date"])
    months = ("OCAK", "ŞUBAT", "MART", "NİSAN", "MAYIS", "HAZİRAN", "TEMMUZ", "AĞUSTOS", "EYLÜL", "EKİM", "KASIM", "ARALIK")
    pharmacies = []
    for raw_name in record["pharmacy"].split(" / "):
        name = raw_name.strip()
        if not name:
            continue
        pharmacies.append(name if "ECZANE" in normalize_tr(name) else f"{name} ECZANESİ")
    return f"{d.day} {months[d.month - 1]}  -  BUGÜNÜN NÖBETÇİ ECZANELERİ:  " + "  -  ".join(pharmacies)


def normalize_tr(value: str) -> str:
    value = value.translate(str.maketrans("İIı", "III")).upper()
    return "".join(ch for ch in unicodedata.normalize("NFD", value) if unicodedata.category(ch) != "Mn")


def make_mirrored_text_gif(text: str, options: dict, width: int, height: int) -> str:
    from PIL import Image, ImageDraw, ImageFont, ImageOps
    color = str(options.get("color", "ffffff"))
    bg = str(options.get("bg_color", "071a35"))
    fg_rgb = tuple(int(color[i:i + 2], 16) for i in (0, 2, 4))
    bg_rgb = tuple(int(bg[i:i + 2], 16) for i in (0, 2, 4))
    font_path = options.get("font", "")
    candidates = [font_path, r"C:\Windows\Fonts\arial.ttf", r"C:\Windows\Fonts\segoeui.ttf"]
    font = None
    for candidate in candidates:
        if candidate and Path(candidate).is_file():
            try:
                font = ImageFont.truetype(candidate, max(8, height - 2))
                break
            except OSError:
                pass
    if font is None:
        font = ImageFont.load_default()
    probe = Image.new("RGB", (1, 1))
    draw = ImageDraw.Draw(probe)
    bounds = draw.textbbox((0, 0), text, font=font)
    text_width = max(1, bounds[2] - bounds[0])
    text_height = max(1, bounds[3] - bounds[1])
    canvas = Image.new("RGB", (text_width + width * 2, height), bg_rgb)
    draw = ImageDraw.Draw(canvas)
    y = max(0, (height - text_height) // 2 - bounds[1])
    draw.text((width - bounds[0], y), text, font=font, fill=fg_rgb)
    static = options.get("animation", 1) == 0
    if static:
        left = max(0, width + (text_width - width) // 2)
        frames = [canvas.crop((left, 0, left + width, height))]
    else:
        total = text_width + width
        frame_count = min(240, max(2, total // 2 + 1))
        direction = range(frame_count - 1, -1, -1) if options.get("animation") == 2 else range(frame_count)
        frames = []
        for index in direction:
            left = round(index * total / max(1, frame_count - 1))
            frame = canvas.crop((left, 0, left + width, height))
            frames.append(frame)
    if options.get("mirror_horizontal"):
        frames = [ImageOps.mirror(frame) for frame in frames]
    if options.get("mirror_vertical"):
        frames = [ImageOps.flip(frame) for frame in frames]
    speed = max(1, min(100, int(options.get("speed", 70))))
    duration = max(25, min(150, 160 - speed))
    out = Path(tempfile.gettempdir()) / f"nobet-mirror-{os.getpid()}.gif"
    frames[0].save(out, format="GIF", save_all=True, append_images=frames[1:], duration=duration, loop=0, disposal=2, optimize=True)
    return str(out)


MONTH_NUMBERS = {
    "OCAK": 1, "SUBAT": 2, "MART": 3, "NISAN": 4, "MAYIS": 5, "HAZIRAN": 6,
    "TEMMUZ": 7, "AGUSTOS": 8, "EYLUL": 9, "EKIM": 10, "KASIM": 11, "ARALIK": 12,
}
ORIENTATION_LABELS = {
    0: "0 · Normal",
    1: "1 · 90°",
    2: "2 · 180°",
    3: "3 · 270°",
}


def parse_xlsx_roster_bytes(filename: str, content: bytes) -> list[dict]:
    """Read the monthly roster layout: date in A, pharmacies in B/C."""
    with zipfile.ZipFile(io.BytesIO(content)) as archive:
        workbook = ET.fromstring(archive.read("xl/workbook.xml"))
        rels = ET.fromstring(archive.read("xl/_rels/workbook.xml.rels"))
        rel_ns = "{http://schemas.openxmlformats.org/package/2006/relationships}"
        sheet_rel = workbook.find(".//{http://schemas.openxmlformats.org/spreadsheetml/2006/main}sheet")
        if sheet_rel is None:
            raise ValueError("Excel dosyasında çalışma sayfası bulunamadı.")
        rel_id = sheet_rel.attrib.get("{http://schemas.openxmlformats.org/officeDocument/2006/relationships}id")
        target = next((item.attrib.get("Target") for item in rels.findall(f"{rel_ns}Relationship") if item.attrib.get("Id") == rel_id), None)
        if not target:
            raise ValueError("Excel çalışma sayfası bağlantısı okunamadı.")
        sheet_path = target.lstrip("/") if target.startswith("/") else "xl/" + target
        sheet_path = sheet_path.replace("xl/xl/", "xl/")
        shared = []
        if "xl/sharedStrings.xml" in archive.namelist():
            strings_root = ET.fromstring(archive.read("xl/sharedStrings.xml"))
            ns_main = "{http://schemas.openxmlformats.org/spreadsheetml/2006/main}"
            shared = ["".join(t.text or "" for t in item.iter(f"{ns_main}t")) for item in strings_root.findall(f"{ns_main}si")]
        sheet = ET.fromstring(archive.read(sheet_path))
    ns = "{http://schemas.openxmlformats.org/spreadsheetml/2006/main}"

    def get_cell(cell):
        if cell is None:
            return ""
        value = cell.find(f"{ns}v")
        if cell.attrib.get("t") == "inlineStr":
            return "".join(t.text or "" for t in cell.iter(f"{ns}t"))
        if value is None or value.text is None:
            return ""
        if cell.attrib.get("t") == "s":
            try:
                return shared[int(value.text)]
            except (ValueError, IndexError):
                return ""
        return value.text

    month_name = ""
    year = None
    for cell in sheet.iter(f"{ns}c"):
        if cell.attrib.get("r") == "B2":
            title = str(get_cell(cell))
            match = re.search(r"(20\d{2})\s+(.+)", title)
            if match:
                year = int(match.group(1))
                month_name = normalize_tr(match.group(2).strip())
            break
    if not year:
        match = re.search(r"(20\d{2})", filename)
        year = int(match.group(1)) if match else date.today().year
    if not month_name:
        stem = normalize_tr(Path(filename).stem)
        month_name = next((month for month in MONTH_NUMBERS if month in stem), "")
    month = MONTH_NUMBERS.get(month_name)
    if not month:
        raise ValueError(f"Ay başlığı anlaşılamadı: {filename}")

    rows = []
    for row in sheet.findall(f".//{ns}sheetData/{ns}row"):
        values = {}
        for cell in row.findall(f"{ns}c"):
            ref = cell.attrib.get("r", "")
            col = "".join(ch for ch in ref if ch.isalpha()).upper()
            values[col] = str(get_cell(cell)).strip()
        day_cell = values.get("A", "")
        day_match = re.match(r"\s*(\d+)\s*-", day_cell)
        if not day_match:
            continue
        row_year, row_month = year, month
        explicit = re.search(r"\(([^)]*)\)", day_cell)
        if explicit:
            note = normalize_tr(explicit.group(1))
            explicit_month = next((m for m in MONTH_NUMBERS if m in note), None)
            explicit_year = re.search(r"20\d{2}", explicit.group(1))
            if explicit_month:
                row_month = MONTH_NUMBERS[explicit_month]
                row_year = int(explicit_year.group(0)) if explicit_year else year + (1 if row_month < month else 0)
        try:
            current_date = date(row_year, row_month, int(day_match.group(1)))
        except ValueError:
            continue
        pharmacies = [v for v in (values.get("B", ""), values.get("C", "")) if v]
        if pharmacies:
            rows.append({"date": current_date.isoformat(), "pharmacies": pharmacies, "note": values.get("D", "")})
    return rows


class PharmacyPanelApp:
    def __init__(self, root: tk.Tk):
        self.root = root
        self.root.title(APP_TITLE)
        self.root.geometry("1280x930")
        self.root.minsize(1080, 820)
        self.root.configure(bg="#f2f5f9")
        self.records: list[dict] = []
        self.slot_manifest: dict[str, dict] = {}
        self.settings = {
            "device_name": "",
            "device_address": "",
            "brightness": 70,
            "animation": "Sola kaydır",
            "speed": 70,
            "text_color": "#ffffff",
            "background_color": "#071a35",
            "font": "UNIFONT",
            "auto_send": True,
            "last_auto_date": "",
            "own_pharmacy_name": "",
            "own_duty_message_enabled": False,
            "save_to_slot": True,
            "save_slot": 1,
            "orientation": 0,
            "mirror_horizontal": False,
        }
        self.busy = False
        self.preview_offset = 0
        self.preview_phase = 0
        self.view_month = date.today().replace(day=1)
        self.status_queue: queue.Queue = queue.Queue()
        self._load()
        if "--minimized" in sys.argv:
            self.settings["auto_send"] = True
        self._style()
        self._build_ui()
        self._refresh_table()
        self._refresh_calendar()
        self._select_today_record()
        self._preview_tick()
        self._poll_status()
        self._auto_device_online = False
        self._auto_reconnect_pending = False
        self._auto_scan_busy = False
        self._check_daily_send()
        self.root.protocol("WM_DELETE_WINDOW", self._close)
        if "--minimized" in sys.argv:
            self.root.after(700, self.root.iconify)

    def _load(self):
        try:
            saved = json.loads(DATA_PATH.read_text(encoding="utf-8"))
            self.records = [canonical_record(x) for x in saved.get("records", [])]
            self.slot_manifest = {str(k): v for k, v in saved.get("slots", {}).items()}
            self.settings.update(saved.get("settings", {}))
        except FileNotFoundError:
            pass
        except Exception as exc:
            messagebox.showwarning(APP_TITLE, f"Kayıt dosyası okunamadı; boş listeyle açılıyor.\n{exc}")

    def _save(self):
        payload = {"records": self.records, "settings": self.settings, "slots": self.slot_manifest}
        temporary = DATA_PATH.with_suffix(".tmp")
        temporary.write_text(json.dumps(payload, ensure_ascii=False, indent=2), encoding="utf-8")
        temporary.replace(DATA_PATH)

    def _style(self):
        style = ttk.Style()
        try:
            style.theme_use("clam")
        except tk.TclError:
            pass
        style.configure("TFrame", background="#f2f5f9")
        style.configure("Card.TFrame", background="#ffffff")
        style.configure("TLabel", background="#f2f5f9", foreground="#17253b", font=("Segoe UI", 10))
        style.configure("Card.TLabel", background="#ffffff", foreground="#17253b", font=("Segoe UI", 10))
        style.configure("Title.TLabel", font=("Segoe UI Semibold", 20), foreground="#10213a")
        style.configure("Sub.TLabel", foreground="#61718a", font=("Segoe UI", 9))
        style.configure("TButton", font=("Segoe UI Semibold", 10), padding=(10, 7))
        style.configure("Primary.TButton", background="#155eef", foreground="#ffffff")
        style.map("Primary.TButton", background=[("active", "#0e4bd2")])
        style.configure("Treeview", rowheight=28, font=("Segoe UI", 9), background="#ffffff", fieldbackground="#ffffff")
        style.configure("Treeview.Heading", font=("Segoe UI Semibold", 9))

    def _build_ui(self):
        outer = ttk.Frame(self.root, padding=18)
        outer.pack(fill="both", expand=True)
        top = ttk.Frame(outer)
        top.pack(fill="x", pady=(0, 12))
        ttk.Label(top, text="Nöbetçi Eczane Paneli", style="Title.TLabel").pack(side="left")
        ttk.Label(top, text="CATA CT-4568  •  BLE  •  96 × 16", style="Sub.TLabel").pack(side="right", pady=(9, 0))

        body = ttk.Frame(outer)
        body.pack(fill="both", expand=True)
        left = ttk.Frame(body, style="Card.TFrame", padding=14)
        left.pack(side="left", fill="both", expand=True, padx=(0, 10))
        right = ttk.Frame(body, style="Card.TFrame", padding=14, width=360)
        right.pack(side="right", fill="y")
        right.pack_propagate(False)
        right_canvas = tk.Canvas(right, bg="#ffffff", highlightthickness=0, width=330)
        right_scroll = ttk.Scrollbar(right, orient="vertical", command=right_canvas.yview)
        right_canvas.configure(yscrollcommand=right_scroll.set)
        right_scroll.pack(side="right", fill="y")
        right_canvas.pack(side="left", fill="both", expand=True)
        panel_controls = ttk.Frame(right_canvas, style="Card.TFrame", padding=4)
        panel_window = right_canvas.create_window((0, 0), window=panel_controls, anchor="nw")
        panel_controls.bind("<Configure>", lambda _event: right_canvas.configure(scrollregion=right_canvas.bbox("all")))
        right_canvas.bind("<Configure>", lambda event: right_canvas.itemconfigure(panel_window, width=event.width))

        ttk.Label(left, text="Nöbet listesi", style="Card.TLabel", font=("Segoe UI Semibold", 13)).pack(anchor="w")
        ttk.Label(left, text="Bir tarih seçin veya aşağıdaki biçimde toplu liste yapıştırın.", style="Card.TLabel").pack(anchor="w", pady=(2, 9))

        self.schedule_tabs = ttk.Notebook(left)
        self.schedule_tabs.pack(fill="both", expand=True)
        self.calendar_tab = ttk.Frame(self.schedule_tabs, padding=6)
        self.list_tab = ttk.Frame(self.schedule_tabs, padding=6)
        self.slots_tab = ttk.Frame(self.schedule_tabs, padding=6)
        self.advanced_tab = ttk.Frame(self.schedule_tabs, padding=6)
        self.schedule_tabs.add(self.calendar_tab, text="Aylık takvim")
        self.schedule_tabs.add(self.list_tab, text="Tüm kayıtlar")
        self.schedule_tabs.add(self.slots_tab, text="Cihaz slotları")
        self.schedule_tabs.add(self.advanced_tab, text="iPixel komutları")
        self._build_calendar_tab()
        self._build_slots_tab()
        self._build_advanced_tab()

        table_frame = ttk.Frame(self.list_tab, style="Card.TFrame")
        table_frame.pack(fill="both", expand=True)
        cols = ("date", "pharmacy")
        self.tree = ttk.Treeview(table_frame, columns=cols, show="headings", height=10, selectmode="browse")
        for key, title, width in (("date", "Tarih", 120), ("pharmacy", "Nöbetçi eczaneler", 620)):
            self.tree.heading(key, text=title)
            self.tree.column(key, width=width, minwidth=70, anchor="w")
        yscroll = ttk.Scrollbar(table_frame, orient="vertical", command=self.tree.yview)
        self.tree.configure(yscrollcommand=yscroll.set)
        self.tree.pack(side="left", fill="both", expand=True)
        yscroll.pack(side="right", fill="y")
        self.tree.bind("<<TreeviewSelect>>", self._on_select)

        form = ttk.Frame(left, style="Card.TFrame")
        form.pack(fill="x", pady=(12, 2))
        self.date_var = tk.StringVar(value=date.today().isoformat())
        self.pharmacy_var = tk.StringVar()
        self.free_mode_var = tk.BooleanVar(value=bool(self.settings.get("free_mode", False)))
        self.free_text_var = tk.StringVar(value=str(self.settings.get("free_text", "")))
        self.date_var.trace_add("write", lambda *_: self._update_preview())
        self.pharmacy_var.trace_add("write", lambda *_: self._update_preview())
        self._field(form, "Tarih (YYYY-AA-GG)", self.date_var, 16, 0, 0)
        self._field(form, "Nöbetçi eczaneler (birden fazla adı / ile ayırın)", self.pharmacy_var, 78, 1, 0, colspan=3)
        buttons = ttk.Frame(form, style="Card.TFrame")
        buttons.grid(row=2, column=0, columnspan=3, sticky="e", padx=8, pady=7)
        ttk.Button(buttons, text="Nöbet listesine ekle / güncelle", command=self.save_form).pack(side="left", padx=3)
        ttk.Button(buttons, text="Seçileni sil", command=self.delete_selected).pack(side="left", padx=3)
        ttk.Button(buttons, text="Bugün", command=self.set_today).pack(side="left", padx=3)

        free_box = ttk.Frame(left, style="Card.TFrame")
        free_box.pack(fill="x", pady=(3, 0))
        ttk.Checkbutton(free_box, text="Serbest metin gönder (nöbet formatı kullanma)", variable=self.free_mode_var, command=self._free_mode_changed).pack(anchor="w", padx=5)
        self.free_text_entry = ttk.Entry(free_box, textvariable=self.free_text_var)
        self.free_text_entry.pack(fill="x", padx=5, pady=(2, 5))
        self.free_text_var.trace_add("write", lambda *_: self._update_preview())

        ttk.Separator(left).pack(fill="x", pady=11)
        bulk_title = ttk.Frame(left, style="Card.TFrame")
        bulk_title.pack(fill="x")
        ttk.Label(bulk_title, text="Toplu liste yapıştır", style="Card.TLabel", font=("Segoe UI Semibold", 11)).pack(side="left")
        ttk.Button(bulk_title, text="Aylık Excel / ZIP içe al", command=self.import_monthly).pack(side="right")
        ttk.Label(left, text="Satır: YYYY-AA-GG | Eczane adı 1 | Eczane adı 2…   •   Adres yazmayın; Excel/ZIP: tarih A, eczane adları B-C", style="Sub.TLabel").pack(anchor="w", pady=(2, 5))
        paste_row = ttk.Frame(left, style="Card.TFrame")
        paste_row.pack(fill="x")
        self.bulk_text = tk.Text(paste_row, height=5, wrap="word", font=("Consolas", 9), relief="solid", bd=1)
        self.bulk_text.pack(side="left", fill="both", expand=True)
        ttk.Button(paste_row, text="Listeyi içe al", command=self.import_bulk).pack(side="right", padx=(9, 0), anchor="n")
        ttk.Button(paste_row, text="CSV aç", command=self.import_csv).pack(side="right", padx=(0, 6), anchor="n")
        ttk.Button(left, text="CSV dışa aktar", command=self.export_csv).pack(anchor="e", pady=(6, 0))

        self._build_panel_controls(panel_controls)

        self.status_var = tk.StringVar(value=f"Hazır  •  Yerel kayıt: {DATA_PATH}")
        status = ttk.Label(outer, textvariable=self.status_var, style="Sub.TLabel", anchor="w")
        status.pack(fill="x", pady=(10, 0))

    def _build_calendar_tab(self):
        bar = ttk.Frame(self.calendar_tab)
        bar.pack(fill="x", pady=(0, 6))
        ttk.Button(bar, text="‹ Önceki ay", command=lambda: self._change_month(-1)).pack(side="left")
        self.month_title = ttk.Label(bar, text="", style="Card.TLabel", font=("Segoe UI Semibold", 12))
        self.month_title.pack(side="left", expand=True)
        ttk.Button(bar, text="Sonraki ay ›", command=lambda: self._change_month(1)).pack(side="right")
        self.calendar_grid = ttk.Frame(self.calendar_tab)
        self.calendar_grid.pack(fill="both", expand=True)
        for col, label in enumerate(("Pzt", "Sal", "Çar", "Per", "Cum", "Cmt", "Paz")):
            ttk.Label(self.calendar_grid, text=label, anchor="center", style="Sub.TLabel").grid(row=0, column=col, sticky="ew", padx=2, pady=2)
            self.calendar_grid.columnconfigure(col, weight=1, uniform="calendar")
        for row in range(1, 7):
            self.calendar_grid.rowconfigure(row, weight=1, uniform="calendar")
        self.calendar_buttons = []

    def _change_month(self, offset):
        month_index = self.view_month.year * 12 + self.view_month.month - 1 + offset
        year, month_zero = divmod(month_index, 12)
        self.view_month = date(year, month_zero + 1, 1)
        self._refresh_calendar()

    def _refresh_calendar(self):
        if not hasattr(self, "calendar_grid"):
            return
        for button in self.calendar_buttons:
            button.destroy()
        self.calendar_buttons.clear()
        month_names = ("Ocak", "Şubat", "Mart", "Nisan", "Mayıs", "Haziran", "Temmuz", "Ağustos", "Eylül", "Ekim", "Kasım", "Aralık")
        self.month_title.configure(text=f"{month_names[self.view_month.month - 1]} {self.view_month.year}")
        month_records = {parse_date(r["date"]).day: r for r in self.records if parse_date(r["date"]).year == self.view_month.year and parse_date(r["date"]).month == self.view_month.month}
        offset = calendar.monthrange(self.view_month.year, self.view_month.month)[0]
        days = calendar.monthrange(self.view_month.year, self.view_month.month)[1]
        today_iso = date.today().isoformat()
        selected_iso = ""
        try:
            selected_iso = parse_date(self.date_var.get()).isoformat()
        except ValueError:
            pass
        for day in range(1, days + 1):
            index = offset + day - 1
            row, col = divmod(index, 7)
            iso = date(self.view_month.year, self.view_month.month, day).isoformat()
            record = month_records.get(day)
            text = str(day)
            if record:
                names = record["pharmacy"].split(" / ")
                short = " / ".join(name[:13] for name in names)
                text += f"\n{short}"
            style = "SelectedDay.TButton" if iso == selected_iso else ("Today.TButton" if iso == today_iso else "TButton")
            button = ttk.Button(self.calendar_grid, text=text, style=style, command=lambda value=iso: self._select_calendar_date(value))
            button.grid(row=row + 1, column=col, sticky="nsew", padx=2, pady=2)
            self.calendar_buttons.append(button)

    def _select_calendar_date(self, iso_date):
        record = next((r for r in self.records if r["date"] == iso_date), None)
        self.date_var.set(iso_date)
        self.pharmacy_var.set(record["pharmacy"] if record else "")
        self._refresh_calendar()
        self._update_preview()

    def _build_slots_tab(self):
        ttk.Label(self.slots_tab, text="Cihazdaki kayıtlar", font=("Segoe UI Semibold", 13)).pack(anchor="w")
        ttk.Label(self.slots_tab, text="Bu BLE kütüphanesi panelden slot adlarını/içeriğini okuyamıyor. Aşağıda uygulamanın gönderdiği içerikler gösterilir; bilinmeyen slotları panelden doğrulayamayız.", wraplength=760).pack(anchor="w", pady=(3, 9))
        self.slot_tree = ttk.Treeview(self.slots_tab, columns=("slot", "content", "updated"), show="headings", height=12)
        for key, title, width in (("slot", "Slot", 90), ("content", "Uygulamanın bildiği içerik", 560), ("updated", "Son gönderim", 150)):
            self.slot_tree.heading(key, text=title)
            self.slot_tree.column(key, width=width, anchor="w")
        self.slot_tree.pack(fill="both", expand=True)
        actions = ttk.Frame(self.slots_tab)
        actions.pack(fill="x", pady=(8, 0))
        ttk.Button(actions, text="Listeyi yenile", command=self._refresh_slots).pack(side="left", padx=(0, 5))
        ttk.Button(actions, text="Seçili slotu göster", command=self.show_selected_slot).pack(side="left", padx=5)
        ttk.Button(actions, text="Seçili slotu sil", command=self.delete_selected_slot).pack(side="left", padx=5)
        ttk.Button(actions, text="Tüm cihaz belleğini temizle…", command=self.clear_device_memory).pack(side="right")
        self._refresh_slots()

    def _refresh_slots(self):
        if not hasattr(self, "slot_tree"):
            return
        for item in self.slot_tree.get_children():
            self.slot_tree.delete(item)
        for slot in range(1, 11):
            info = self.slot_manifest.get(str(slot), {})
            content = info.get("text", "Bilinmeyen / uygulamayla kaydedilmemiş")
            updated = info.get("updated", "—")
            self.slot_tree.insert("", "end", iid=f"slot-{slot}", values=(slot, content, updated))

    def _selected_slot(self):
        sel = self.slot_tree.selection()
        if not sel:
            messagebox.showinfo(APP_TITLE, "Önce bir slot seçin.")
            return None
        return int(sel[0].split("-")[1])

    def show_selected_slot(self):
        slot = self._selected_slot()
        if slot is None:
            return
        if str(slot) not in self.slot_manifest and not messagebox.askyesno(APP_TITLE, "Bu slotun içeriği bilinmiyor. Boşsa panel diğer slotlar arasında dolaşabilir. Yine de gösterilsin mi?"):
            return
        self._background(self._run_command_async("show_slot", {"number": slot}), f"Slot {slot} gösteriliyor…", lambda _: self._set_status(f"Slot {slot} gösteriliyor."))

    def delete_selected_slot(self):
        slot = self._selected_slot()
        if slot is None or not messagebox.askyesno(APP_TITLE, f"Cihazdaki {slot} numaralı slot silinsin mi?"):
            return
        self._background(self._run_command_async("delete", {"n": slot}), f"Slot {slot} siliniyor…", lambda _: self._slot_deleted(slot))

    def _slot_deleted(self, slot):
        self.slot_manifest.pop(str(slot), None)
        self._save()
        self._refresh_slots()
        self._set_status(f"Slot {slot} silindi.")

    def clear_device_memory(self):
        if not messagebox.askyesno(APP_TITLE, "Cihaz belleğindeki TÜM kayıtlar ve bazı cihaz ayarları silinecek. Bu işlem geri alınamaz. Devam edilsin mi?"):
            return
        self._background(self._run_command_async("clear", {}), "Cihaz belleği temizleniyor…", self._memory_cleared)

    def _memory_cleared(self, _result):
        self.slot_manifest.clear()
        self._save()
        self._refresh_slots()
        self._set_status("Cihaz belleği temizlendi. Ekran ve cihaz ayarları varsayılanlara dönebilir.")

    def _build_advanced_tab(self):
        ttk.Label(self.advanced_tab, text="Gelişmiş iPixel komutları", font=("Segoe UI Semibold", 13)).pack(anchor="w")
        ttk.Label(self.advanced_tab, text="Kütüphanedeki komutları JSON parametreleriyle çalıştırır. Komut ve parametreler cihaza gönderilir; clear komutu cihaz belleğini tamamen siler.", wraplength=760).pack(anchor="w", pady=(3, 10))
        row = ttk.Frame(self.advanced_tab)
        row.pack(fill="x")
        ttk.Label(row, text="Komut").pack(side="left")
        self.advanced_command_var = tk.StringVar(value="set_orientation")
        commands = ("set_orientation", "set_brightness", "set_power", "set_time", "set_clock_mode", "set_schedule", "set_fun_mode", "set_rhythm_mode", "set_rhythm_mode_2", "set_timer", "set_scores", "set_pixel", "show_slot", "delete", "send_image", "send_image_hex", "send_text", "clear", "get_device_info", "version")
        combo = ttk.Combobox(row, textvariable=self.advanced_command_var, values=commands, state="readonly", width=28)
        combo.pack(side="left", padx=8)
        combo.bind("<<ComboboxSelected>>", lambda _event: self._advanced_example())
        ttk.Button(row, text="Komutları çalıştır", command=self.run_advanced_command).pack(side="right")
        self.advanced_args = tk.Text(self.advanced_tab, height=14, wrap="word", font=("Consolas", 10))
        self.advanced_args.pack(fill="both", expand=True, pady=(8, 4))
        self.advanced_help = ttk.Label(self.advanced_tab, text="", style="Sub.TLabel", wraplength=760)
        self.advanced_help.pack(anchor="w")
        self._advanced_example()

    def _advanced_example(self):
        command = self.advanced_command_var.get()
        examples = {
            "set_orientation": ('{"orientation": 2}', "0–3: yön/rotasyon değeri; 2 genellikle 180° dönüş içindir."),
            "set_brightness": ('{"level": 70}', "Parlaklık 0–100."),
            "set_power": ('{"on": true}', "Paneli aç/kapat."),
            "set_time": ('{}', "Bilgisayarın yerel saatini panele yollar."),
            "set_clock_mode": ('{"style": 1, "show_date": true, "format_24": true}', "Saat görünümünü açar; nöbet metninin yerini alır."),
            "set_schedule": ('{"hour": 22, "minute": 0, "on": false, "days": null, "slot": 0}', "Cihazda zamanlı aç/kapat komutu."),
            "set_fun_mode": ('{"enable": false}', "DIY/piksel modunu açıp kapatır."),
            "set_rhythm_mode": ('{"style": 0, "l1": 0, "l2": 0, "l3": 0}', "Mikrofon/ritim efekt ayarları."),
            "set_rhythm_mode_2": ('{"style": 0, "t": 0}', "Alternatif ritim modu."),
            "set_timer": ('{"action": "stop"}', "action: start, pause veya stop."),
            "set_scores": ('{"score_p1": 0, "score_p2": 0}', "Skor ekranı; mevcut yazının yerini alır."),
            "set_pixel": ('{"x": 0, "y": 0, "color": "ff0000"}', "Tek piksel yakar; koordinatlar panel boyutuna bağlıdır."),
            "show_slot": ('{"number": 1}', "Slot boşsa cihaz diğer slotları oynatabilir."),
            "delete": ('{"n": 1}', "Belirtilen tek slotu siler."),
            "send_image": ('{"path": "C:/resim.gif", "save_slot": 0}', "PNG/JPG/GIF dosyası yollar."),
            "send_image_hex": ('{"hex_string": "89504e47...", "file_extension": ".png", "save_slot": 0}', "Dosyayı hex içerik olarak yollar."),
            "send_text": ('{"text": "Merhaba", "animation": 1, "speed": 70, "save_slot": 0}', "Metin göndermek için ana ekrandaki gönder düğmelerini kullanmanız önerilir."),
            "clear": ('{}', "Tüm cihaz belleğini ve bazı ayarları siler."),
            "get_device_info": ('{}', "Panel ölçüsü, yazılım bilgisi ve yetenekleri."),
            "version": ('{}', "PC tarafındaki pypixelcolor kütüphane sürümü."),
        }
        value, help_text = examples.get(command, ("{}", ""))
        self.advanced_args.delete("1.0", "end")
        self.advanced_args.insert("1.0", value)
        self.advanced_help.configure(text=help_text)

    def run_advanced_command(self):
        command = self.advanced_command_var.get()
        try:
            args = json.loads(self.advanced_args.get("1.0", "end"))
            if not isinstance(args, dict):
                raise ValueError("Parametreler bir JSON nesnesi olmalı.")
        except Exception as exc:
            messagebox.showerror(APP_TITLE, f"JSON parametreleri okunamadı: {exc}")
            return
        if command == "clear" and not messagebox.askyesno(APP_TITLE, "Bu komut cihaz belleğindeki TÜM kayıtları ve bazı ayarları siler. Emin misiniz?"):
            return
        if command == "delete" and not messagebox.askyesno(APP_TITLE, f"{args.get('n')} numaralı slot silinsin mi?"):
            return
        self._background(self._run_command_async(command, args), f"{command} komutu gönderiliyor…", lambda result: self._advanced_done(command, args, result))

    def _advanced_done(self, command, args, result):
        if command == "set_orientation":
            value = int(args.get("orientation", 0))
            self.orientation_var.set(ORIENTATION_LABELS.get(value, ORIENTATION_LABELS[0]))
            self.settings["orientation"] = value
        elif command == "delete":
            self._slot_deleted(int(args["n"]))
        elif command == "clear":
            self._memory_cleared(result)
        self._save()
        self._set_status(str(result))

    async def _run_command_async(self, command, args):
        from bleak import BleakScanner
        from pypixelcolor import AsyncClient
        if command == "version":
            from pypixelcolor import __version__
            return f"pypixelcolor {__version__}"
        devices = await BleakScanner.discover(timeout=6)
        compatible = [d for d in devices if d.name and d.name.startswith(DEVICE_PREFIX)]
        preferred = self._current_device_name()
        match = next((d for d in compatible if preferred and d.name == preferred), None) or (compatible[0] if compatible else None)
        if not match:
            raise RuntimeError(f"{DEVICE_PREFIX} ile başlayan uyumlu panel bulunamadı.")
        async with AsyncClient(match.address) as client:
            if command == "get_device_info":
                return str(client.get_device_info())
            method = getattr(client, command)
            await method(**args)
            return f"{command} tamamlandı · {match.name}"

    def _field(self, parent, label, variable, width, row, col, colspan=1):
        box = ttk.Frame(parent, style="Card.TFrame")
        box.grid(row=row, column=col, columnspan=colspan, sticky="ew", padx=4, pady=4)
        ttk.Label(box, text=label, style="Sub.TLabel").pack(anchor="w")
        entry = ttk.Entry(box, textvariable=variable, width=width)
        entry.pack(fill="x", pady=(3, 0))
        parent.columnconfigure(col, weight=1)
        return entry

    def _build_panel_controls(self, panel):
        ttk.Label(panel, text="Panel kontrolü", style="Card.TLabel", font=("Segoe UI Semibold", 13)).pack(anchor="w")
        ttk.Label(panel, text="Panel telefona bağlıysa uygulamayı kapatın; BLE tek bağlantı kabul edebilir.", style="Sub.TLabel", wraplength=320).pack(anchor="w", pady=(3, 10))
        device_row = ttk.Frame(panel, style="Card.TFrame")
        device_row.pack(fill="x")
        self.device_var = tk.StringVar(value=self.settings.get("device_name", ""))
        self.device_combo = ttk.Combobox(device_row, textvariable=self.device_var, values=[], state="normal")
        self.device_combo.pack(side="left", fill="x", expand=True)
        ttk.Button(device_row, text="Tara", command=self.scan_devices).pack(side="left", padx=(6, 0))
        self.device_combo.bind("<<ComboboxSelected>>", self._persist_device_name)


        ttk.Button(panel, text="Panele bağlan", command=self.connect_check).pack(fill="x", pady=(8, 12))
        device_actions = ttk.Frame(panel, style="Card.TFrame")
        device_actions.pack(fill="x", pady=(0, 8))
        ttk.Button(device_actions, text="Saati eşitle", command=lambda: self.run_device_command("time")).pack(side="left", fill="x", expand=True, padx=(0, 3))
        ttk.Button(device_actions, text="Aç", command=lambda: self.run_device_command("power_on")).pack(side="left", fill="x", expand=True, padx=3)
        ttk.Button(device_actions, text="Kapat", command=lambda: self.run_device_command("power_off")).pack(side="left", fill="x", expand=True, padx=(3, 0))
        slot_row = ttk.Frame(panel, style="Card.TFrame")
        slot_row.pack(fill="x", pady=(0, 4))
        self.save_slot_var = tk.IntVar(value=int(self.settings.get("save_slot", 1)))
        self.save_to_slot_var = tk.BooleanVar(value=bool(self.settings.get("save_to_slot", True)))
        ttk.Checkbutton(slot_row, text="Cihaza kalıcı kaydet · slot", variable=self.save_to_slot_var, command=self._save_control_settings).pack(side="left")
        ttk.Spinbox(slot_row, from_=1, to=10, width=4, textvariable=self.save_slot_var, command=self._save_control_settings).pack(side="left", padx=5)
        ttk.Label(panel, text="Kalıcı slot için cihazın Program Listesinde slot tanımlı olmalı.", style="Sub.TLabel", wraplength=320).pack(anchor="w", pady=(0, 5))
        orientation_row = ttk.Frame(panel, style="Card.TFrame")
        orientation_row.pack(fill="x", pady=(3, 3))
        ttk.Label(orientation_row, text="Ekran yönü", style="Card.TLabel").pack(side="left")
        self.orientation_var = tk.StringVar(value=ORIENTATION_LABELS.get(int(self.settings.get("orientation", 0)), ORIENTATION_LABELS[0]))
        self.orientation_combo = ttk.Combobox(orientation_row, textvariable=self.orientation_var, values=list(ORIENTATION_LABELS.values()), state="readonly", width=12)
        self.orientation_combo.pack(side="right")
        self.orientation_combo.bind("<<ComboboxSelected>>", lambda _event: self._save_control_settings())
        ttk.Button(panel, text="Yönü uygula", command=self.apply_orientation).pack(fill="x", pady=(0, 3))
        self.mirror_horizontal_var = tk.BooleanVar(value=bool(self.settings.get("mirror_horizontal", False)))
        self.mirror_vertical_var = tk.BooleanVar(value=bool(self.settings.get("mirror_vertical", False)))
        mirror_row = ttk.Frame(panel, style="Card.TFrame")
        mirror_row.pack(fill="x")
        ttk.Checkbutton(mirror_row, text="Yatay ayna", variable=self.mirror_horizontal_var, command=self._save_control_settings).pack(side="left")
        ttk.Checkbutton(mirror_row, text="Dikey ayna", variable=self.mirror_vertical_var, command=self._save_control_settings).pack(side="left", padx=(10, 0))
        ttk.Label(panel, text="Ayna seçilince metin, animasyonlu görsele çevrilerek yansıtılır.", style="Sub.TLabel", wraplength=320).pack(anchor="w", pady=(0, 4))
        self._preview = tk.Canvas(panel, height=110, bg="#071a35", highlightthickness=0)
        self._preview.pack(fill="x", pady=(0, 12))
        self.preview_label = ttk.Label(panel, text="Önizleme", style="Sub.TLabel")
        self.preview_label.pack(anchor="w")
        ttk.Label(panel, text="Önizleme; seçili tarih, metin, renk, efekt ve hızı yaklaşık gösterir.", style="Sub.TLabel", wraplength=320).pack(anchor="w", pady=(2, 0))

        self.color_button = ttk.Button(panel, text="Yazı rengini seç", command=self.choose_text_color)
        self.color_button.pack(fill="x", pady=(8, 4))
        self.bg_button = ttk.Button(panel, text="Arka plan rengini seç", command=self.choose_bg_color)
        self.bg_button.pack(fill="x", pady=4)
        ttk.Label(panel, text="Yazı efekti", style="Card.TLabel").pack(anchor="w", pady=(9, 2))
        self.animation_var = tk.StringVar(value=self.settings["animation"])
        self.animation_combo = ttk.Combobox(panel, textvariable=self.animation_var, values=list(ANIMATIONS), state="readonly")
        self.animation_combo.pack(fill="x")
        self.animation_combo.bind("<<ComboboxSelected>>", lambda _: self._save_control_settings())

        self.speed_var = tk.IntVar(value=int(self.settings["speed"]))
        self.brightness_var = tk.IntVar(value=int(self.settings["brightness"]))
        self._scale(panel, "Kayma / efekt hızı", self.speed_var, self._save_control_settings)
        self._scale(panel, "Parlaklık", self.brightness_var, self._save_control_settings)
        font_row = ttk.Frame(panel, style="Card.TFrame")
        font_row.pack(fill="x", pady=(9, 3))
        ttk.Label(font_row, text="Yazı tipi", style="Card.TLabel").pack(side="left")
        self.font_var = tk.StringVar(value=str(self.settings.get("font", "UNIFONT")))
        ttk.Button(font_row, text="TTF/OTF seç…", command=self.choose_font).pack(side="right")
        ttk.Label(panel, textvariable=self.font_var, style="Sub.TLabel", wraplength=315).pack(anchor="w")

        self.auto_var = tk.BooleanVar(value=bool(self.settings.get("auto_send", False)))
        ttk.Checkbutton(panel, text="Gün değişince veya panel yeniden bağlanınca gönder", variable=self.auto_var, command=self._save_control_settings).pack(anchor="w", pady=(11, 6))
        ttk.Label(panel, text="Otomatik gönderim için uygulama açık ve panelin Bluetooth menzili içinde olmalı. Panel yaklaşık 20 saniyede bir taranır.", style="Sub.TLabel", wraplength=320).pack(anchor="w")
        self.own_pharmacy_var = tk.StringVar(value=str(self.settings.get("own_pharmacy_name", "")))
        own_row = ttk.Frame(panel, style="Card.TFrame")
        own_row.pack(fill="x", pady=(10, 3))
        ttk.Label(own_row, text="Bizim eczane adımız", style="Card.TLabel").pack(side="left")
        own_entry = ttk.Entry(own_row, textvariable=self.own_pharmacy_var)
        own_entry.pack(side="right", fill="x", expand=True, padx=(8, 0))
        own_entry.bind("<KeyRelease>", lambda _event: self._save_control_settings())
        self.own_duty_message_var = tk.BooleanVar(value=bool(self.settings.get("own_duty_message_enabled", False)))
        ttk.Checkbutton(panel, text="Biz nöbetçiyken ‘BUGÜN NÖBETÇİYİZ’ göster", variable=self.own_duty_message_var, command=self._save_control_settings).pack(anchor="w", pady=(3, 3))
        ttk.Label(panel, text="Kayıttaki eczane adıyla eşleşirse bugünün liste yazısı yerine seçili kaydırma efektiyle bu mesaj gönderilir.", style="Sub.TLabel", wraplength=320).pack(anchor="w")
        self.startup_var = tk.BooleanVar(value=self._startup_enabled())
        ttk.Checkbutton(panel, text="Windows açılışında küçültülmüş başlat", variable=self.startup_var, command=self._toggle_startup).pack(anchor="w", pady=(8, 3))
        ttk.Label(panel, text="Bu seçenek otomatik günlük gönderimi de açar. Uygulama görev çubuğuna küçültülür; BLE taraması sürer.", style="Sub.TLabel", wraplength=320).pack(anchor="w")
        ttk.Button(panel, text="BUGÜNÜ PANELE GÖNDER", style="Primary.TButton", command=self.send_today).pack(fill="x", pady=(15, 7))
        ttk.Button(panel, text="Seçili nöbeti gönder", command=self.send_preview).pack(fill="x", pady=(0, 6))
        ttk.Button(panel, text="Serbest metni gönder", command=self.send_free_text).pack(fill="x")
        ttk.Button(panel, text="Görsel / GIF gönder…", command=self.send_image_file).pack(fill="x", pady=(6, 0))

    def _scale(self, parent, title, variable, callback):
        row = ttk.Frame(parent, style="Card.TFrame")
        row.pack(fill="x", pady=(8, 0))
        ttk.Label(row, text=title, style="Card.TLabel").pack(anchor="w")
        scale = ttk.Scale(row, from_=1, to=100, orient="horizontal", command=lambda value: self._scale_change(variable, value, callback))
        scale.set(variable.get())
        scale.pack(fill="x", pady=(3, 0))

    def _scale_change(self, var, value, callback):
        var.set(int(float(value)))
        callback()

    def _persist_device_name(self, _event=None):
        self.settings["device_name"] = self.device_var.get().strip()
        self._save()

    def _save_control_settings(self):
        self.settings.update({
            "device_name": self.device_var.get().strip(),
            "brightness": self.brightness_var.get(),
            "animation": self.animation_var.get(),
            "speed": self.speed_var.get(),
            "text_color": self.settings.get("text_color", "#ffffff"),
            "background_color": self.settings.get("background_color", "#071a35"),
            "font": self.font_var.get(),
            "auto_send": self.auto_var.get(),
            "own_pharmacy_name": self.own_pharmacy_var.get().strip(),
            "own_duty_message_enabled": self.own_duty_message_var.get(),
            "free_mode": self.free_mode_var.get(),
            "free_text": self.free_text_var.get(),
            "save_to_slot": self.save_to_slot_var.get(),
            "save_slot": max(1, min(10, int(self.save_slot_var.get()))),
            "orientation": self._current_orientation(),
            "mirror_horizontal": self.mirror_horizontal_var.get(),
            "mirror_vertical": self.mirror_vertical_var.get(),
            "startup_enabled": self.startup_var.get(),
        })
        if not self.auto_var.get() and hasattr(self, "_auto_device_online"):
            self._auto_device_online = False
            self._auto_reconnect_pending = True
        self._save()
        self._update_preview()

    def _startup_command(self):
        executable = Path(sys.executable).resolve()
        return f'"{executable}" --minimized'

    def _startup_enabled(self):
        try:
            import winreg
            key_path = r"Software\Microsoft\Windows\CurrentVersion\Run"
            with winreg.OpenKey(winreg.HKEY_CURRENT_USER, key_path, 0, winreg.KEY_QUERY_VALUE) as key:
                command, _ = winreg.QueryValueEx(key, "NobetPanel")
            return "--minimized" in command
        except (OSError, ImportError):
            return False

    def _toggle_startup(self):
        try:
            import winreg
            key_path = r"Software\Microsoft\Windows\CurrentVersion\Run"
            with winreg.CreateKey(winreg.HKEY_CURRENT_USER, key_path) as key:
                if self.startup_var.get():
                    winreg.SetValueEx(key, "NobetPanel", 0, winreg.REG_SZ, self._startup_command())
                    self.auto_var.set(True)
                else:
                    try:
                        winreg.DeleteValue(key, "NobetPanel")
                    except FileNotFoundError:
                        pass
            self._save_control_settings()
            state = "Windows açılışında küçültülmüş başlatma etkin." if self.startup_var.get() else "Windows açılışında başlatma kapatıldı."
            self._set_status(state)
        except Exception as exc:
            self.startup_var.set(False)
            messagebox.showerror(APP_TITLE, f"Windows başlangıç ayarı kaydedilemedi.\n\n{exc}")

    def _current_device_name(self):
        return self.device_var.get().strip()

    def _current_orientation(self):
        try:
            return int(self.orientation_var.get().split("·", 1)[0].strip())
        except (ValueError, AttributeError):
            return 0

    def scan_devices(self):
        if self.busy:
            return
        self._background(self._scan_async(), "Bluetooth cihazları taranıyor…", self._scan_done)

    async def _scan_async(self):
        from bleak import BleakScanner
        found = await BleakScanner.discover(timeout=8)
        return [(d.name or "", d.address) for d in found if d.name and d.name.startswith(DEVICE_PREFIX)]

    def _scan_done(self, result):
        if not result:
            self._set_status("LED_BLE cihazı bulunamadı. Panel açık, telefon Bluetooth bağlantısı kapalı olsun.")
            return
        names = [name for name, _ in result]
        self.device_combo.configure(values=names)
        selected = next((name for name in names if name == self._current_device_name()), names[0])
        self.device_var.set(selected)
        self.settings["device_address"] = next(addr for name, addr in result if name == selected)
        self.settings["device_name"] = selected
        self._save()
        self._set_status(f"Panel bulundu: {selected}")

    def connect_check(self):
        self._background(self._connect_async(self._current_device_name()), "Panele bağlanılıyor ve ekran bilgisi okunuyor…", self._connect_done)

    async def _connect_async(self, device_name):
        from bleak import BleakScanner
        from pypixelcolor import AsyncClient
        devices = await BleakScanner.discover(timeout=7)
        compatible = [d for d in devices if d.name and d.name.startswith(DEVICE_PREFIX)]
        match = next((d for d in compatible if device_name and d.name == device_name), None) or (compatible[0] if compatible else None)
        if not match:
            raise RuntimeError(f"{DEVICE_PREFIX} benzeri bir panel bulunamadı. Panel açık mı? Telefon uygulamasını kapatıp tekrar tarayın.")
        async with AsyncClient(match.address) as client:
            info = client.get_device_info()
            return f"Bağlantı başarılı: {device_name}  •  {info.width} × {info.height} piksel  •  yazılım {info.mcu_version}"

    def _connect_done(self, result):
        self._set_status(result)

    def run_device_command(self, command):
        labels = {"time": "Panel saati bilgisayarla eşitleniyor…", "power_on": "Panel açılıyor…", "power_off": "Panel kapatılıyor…"}
        self._background(self._device_command_async(command), labels[command], lambda result: self._set_status(result))

    def apply_orientation(self):
        orientation = self._current_orientation()
        self._save_control_settings()
        self._background(self._run_command_async("set_orientation", {"orientation": orientation}), f"Ekran yönü {orientation} uygulanıyor…", lambda result: self._set_status(result))

    def send_image_file(self):
        path = filedialog.askopenfilename(title="Panele gönderilecek görsel veya GIF", filetypes=[("Görsel ve animasyon", "*.png *.jpg *.jpeg *.bmp *.webp *.gif *.tiff"), ("Tüm dosyalar", "*.*")])
        if not path:
            return
        options = self._send_options()
        def completed(result):
            self._remember_slot(Path(path).name, options, result)
            self._set_status(f"Görsel panele gönderildi: {Path(path).name}")
        self._background(self._send_image_async(path, options), f"Görsel panele gönderiliyor: {Path(path).name}…", completed)

    async def _send_image_async(self, path, options):
        from bleak import BleakScanner
        from pypixelcolor import AsyncClient
        devices = await BleakScanner.discover(timeout=7)
        compatible = [d for d in devices if d.name and d.name.startswith(DEVICE_PREFIX)]
        preferred = options.get("device_name", "")
        match = next((d for d in compatible if preferred and d.name == preferred), None) or (compatible[0] if compatible else None)
        if not match:
            raise RuntimeError(f"{DEVICE_PREFIX} ile başlayan uyumlu panel bulunamadı.")
        transformed = self._transform_image(path, options)
        try:
            async with AsyncClient(match.address) as client:
                await client.set_orientation(options.get("orientation", 0))
                await client.send_image(transformed, resize_method="fit", save_slot=options.get("save_slot", 0))
                if options.get("save_slot", 0) > 0:
                    await client.send_image(transformed, resize_method="fit", save_slot=0)
        finally:
            if transformed != path:
                Path(transformed).unlink(missing_ok=True)
        return {"device": match.name, "save_slot": options.get("save_slot", 0)}

    def _transform_image(self, path, options):
        if not options.get("mirror_horizontal") and not options.get("mirror_vertical"):
            return path
        from PIL import Image, ImageOps, ImageSequence
        source = Image.open(path)
        def transform(frame):
            out = frame.convert("RGB")
            if options.get("mirror_horizontal"):
                out = ImageOps.mirror(out)
            if options.get("mirror_vertical"):
                out = ImageOps.flip(out)
            return out
        if getattr(source, "n_frames", 1) > 1:
            frames, durations = [], []
            for frame in ImageSequence.Iterator(source):
                frames.append(transform(frame.copy()))
                durations.append(frame.info.get("duration", source.info.get("duration", 100)))
            output = Path(tempfile.gettempdir()) / f"nobet-mirror-{os.getpid()}.gif"
            frames[0].save(output, format="GIF", save_all=True, append_images=frames[1:], duration=durations, loop=source.info.get("loop", 0), disposal=2, optimize=True)
        else:
            output = Path(tempfile.gettempdir()) / f"nobet-mirror-{os.getpid()}.png"
            transform(source).save(output, format="PNG")
        return str(output)

    async def _device_command_async(self, command):
        from bleak import BleakScanner
        from pypixelcolor import AsyncClient
        found = await BleakScanner.discover(timeout=6)
        compatible = [d for d in found if d.name and d.name.startswith(DEVICE_PREFIX)]
        preferred = self._current_device_name()
        match = next((d for d in compatible if preferred and d.name == preferred), None) or (compatible[0] if compatible else None)
        if not match:
            raise RuntimeError(f"{DEVICE_PREFIX} ile başlayan uyumlu panel bulunamadı.")
        async with AsyncClient(match.address) as client:
            if command == "time":
                await client.set_time()
                return f"Panel saati eşitlendi: {match.name}"
            await client.set_power(command == "power_on")
            return f"Panel {'açıldı' if command == 'power_on' else 'kapatıldı'}: {match.name}"

    def save_form(self):
        try:
            record = canonical_record({"date": self.date_var.get(), "pharmacy": self.pharmacy_var.get()})
        except Exception as exc:
            messagebox.showerror(APP_TITLE, str(exc))
            return
        self.records = [r for r in self.records if r["date"] != record["date"]]
        self.records.append(record)
        self.records.sort(key=lambda r: r["date"])
        self._save()
        self._refresh_table()
        self._refresh_calendar()
        self._select_date_in_table(record["date"])
        self._update_preview()
        self._set_status(f"Kaydedildi: {record['date']} — {record['pharmacy']}")

    def delete_selected(self):
        selection = self.tree.selection()
        if not selection:
            messagebox.showinfo(APP_TITLE, "Önce listeden bir gün seçin.")
            return
        selected_date = self.tree.item(selection[0], "values")[0]
        selected_date = parse_date(selected_date).isoformat()
        self.records = [r for r in self.records if r["date"] != selected_date]
        self._save()
        self._refresh_table()
        self._clear_form()
        self._update_preview()

    def _refresh_table(self):
        for item in self.tree.get_children():
            self.tree.delete(item)
        for rec in sorted(self.records, key=lambda r: r["date"]):
            d = parse_date(rec["date"]).strftime("%d.%m.%Y")
            self.tree.insert("", "end", values=(d, rec["pharmacy"]))
        if hasattr(self, "calendar_grid"):
            self._refresh_calendar()

    def _on_select(self, _event=None):
        selection = self.tree.selection()
        if not selection:
            return
        values = self.tree.item(selection[0], "values")
        self.date_var.set(parse_date(values[0]).isoformat())
        self.pharmacy_var.set(values[1])
        try:
            d = parse_date(values[0])
            self.view_month = d.replace(day=1)
            self._refresh_calendar()
        except ValueError:
            pass
        self._update_preview()

    def _select_date_in_table(self, iso_date):
        for item in self.tree.get_children():
            if parse_date(self.tree.item(item, "values")[0]).isoformat() == iso_date:
                self.tree.selection_set(item)
                self.tree.see(item)
                break

    def _select_today_record(self):
        self.view_month = date.today().replace(day=1)
        self.date_var.set(date.today().isoformat())
        self._select_date_in_table(date.today().isoformat())
        self._update_preview()

    def _clear_form(self):
        self.pharmacy_var.set("")

    def set_today(self):
        self.view_month = date.today().replace(day=1)
        self.date_var.set(date.today().isoformat())
        self._select_date_in_table(date.today().isoformat())
        self._refresh_calendar()

    def import_bulk(self):
        raw = self.bulk_text.get("1.0", "end").strip()
        if not raw:
            messagebox.showinfo(APP_TITLE, "Önce listeyi metin alanına yapıştırın.")
            return
        imported, errors = [], []
        for line_no, line in enumerate(raw.splitlines(), 1):
            line = line.strip()
            if not line or line.startswith("#"):
                continue
            if any(h in line.lower() for h in ("tarih", "eczane adı", "eczane adi")) and line_no == 1:
                continue
            delimiter = "\t" if "\t" in line else ("|" if "|" in line else (";" if ";" in line else ","))
            parts = [p.strip().strip('"') for p in line.split(delimiter)]
            if len(parts) < 2 or not parts[1:]:
                errors.append(f"{line_no}. satır: tarih ve eczane adı bulunamadı")
                continue
            try:
                pharmacies = [part for part in parts[1:] if part]
                imported.append(canonical_record({"date": parts[0], "pharmacy": " / ".join(pharmacies)}))
            except Exception as exc:
                errors.append(f"{line_no}. satır: {exc}")
        if not imported:
            messagebox.showerror(APP_TITLE, "İçe alınabilir satır bulunamadı.\n" + "\n".join(errors[:8]))
            return
        by_date = {r["date"]: r for r in self.records}
        by_date.update({r["date"]: r for r in imported})
        self.records = sorted(by_date.values(), key=lambda r: r["date"])
        self._save()
        self._refresh_table()
        self._select_date_in_table(date.today().isoformat())
        self._update_preview()
        summary = f"{len(imported)} kayıt içe alındı."
        if errors:
            summary += "\nAtlanan satırlar:\n" + "\n".join(errors[:8])
        messagebox.showinfo(APP_TITLE, summary)

    def import_csv(self):
        path = filedialog.askopenfilename(title="Nöbet listesi CSV dosyasını seçin", filetypes=[("CSV dosyaları", "*.csv"), ("Metin dosyaları", "*.txt"), ("Tüm dosyalar", "*.*")])
        if not path:
            return
        try:
            content = Path(path).read_text(encoding="utf-8-sig")
        except Exception as exc:
            messagebox.showerror(APP_TITLE, f"Dosya açılamadı: {exc}")
            return
        self.bulk_text.delete("1.0", "end")
        self.bulk_text.insert("1.0", content)
        self.import_bulk()

    def import_monthly(self):
        path = filedialog.askopenfilename(
            title="Aylık nöbet Excel dosyası veya ZIP arşivi seçin",
            filetypes=[("Excel veya ZIP", "*.xlsx *.zip"), ("Tüm dosyalar", "*.*")],
        )
        if not path:
            return
        try:
            archive_path = Path(path)
            if archive_path.suffix.lower() == ".zip":
                with zipfile.ZipFile(archive_path) as archive:
                    files = [(name, archive.read(name)) for name in archive.namelist() if name.lower().endswith(".xlsx") and not name.startswith("__MACOSX/")]
            else:
                files = [(archive_path.name, archive_path.read_bytes())]
            imported = []
            for name, content in files:
                imported.extend(parse_xlsx_roster_bytes(Path(name).name, content))
            if not imported:
                raise ValueError("Seçilen Excel dosyalarında A sütununda gün ve B/C sütunlarında eczane adı olan kayıt bulunamadı.")
            by_date = {r["date"]: r for r in self.records}
            for item in imported:
                pharmacy = " / ".join(item["pharmacies"])
                by_date[item["date"]] = canonical_record({"date": item["date"], "pharmacy": pharmacy})
            self.records = sorted(by_date.values(), key=lambda r: r["date"])
            self._save()
            self._refresh_table()
            self._select_date_in_table(date.today().isoformat())
            self._update_preview()
            summary = f"{len(imported)} günlük nöbet kaydı içe alındı. Aynı tarihteki iki eczane adı birlikte korundu.\n\nGönderim metninde adres kullanılmaz."
            messagebox.showinfo(APP_TITLE, summary)
        except Exception as exc:
            messagebox.showerror(APP_TITLE, f"Aylık liste okunamadı.\n\n{exc}")

    def export_csv(self):
        if not self.records:
            messagebox.showinfo(APP_TITLE, "Dışa aktarılacak kayıt yok.")
            return
        path = filedialog.asksaveasfilename(title="Nöbet listesini kaydet", defaultextension=".csv", filetypes=[("CSV dosyaları", "*.csv")], initialfile="nobet_listesi.csv")
        if not path:
            return
        with open(path, "w", newline="", encoding="utf-8-sig") as f:
            writer = csv.writer(f, delimiter=";")
            writer.writerow(["Tarih", "Nöbetçi eczaneler"])
            for r in self.records:
                writer.writerow([r["date"], r["pharmacy"]])
        self._set_status(f"CSV dışa aktarıldı: {path}")

    def choose_text_color(self):
        chosen = colorchooser.askcolor(self.settings.get("text_color", "#ffffff"), title="Yazı rengi seç")
        if chosen and chosen[1]:
            self.settings["text_color"] = chosen[1]
            self._save_control_settings()

    def choose_bg_color(self):
        chosen = colorchooser.askcolor(self.settings.get("background_color", "#071a35"), title="Arka plan rengi seç")
        if chosen and chosen[1]:
            self.settings["background_color"] = chosen[1]
            self._save_control_settings()
            self._preview.configure(bg=chosen[1])

    def choose_font(self):
        path = filedialog.askopenfilename(title="Yazı tipi dosyasını seç", filetypes=[("Font dosyaları", "*.ttf *.otf"), ("Tüm dosyalar", "*.*")])
        if path:
            self.font_var.set(path)
            self._save_control_settings()

    def _selected_record(self, for_today=False):
        iso = date.today().isoformat() if for_today else parse_date(self.date_var.get()).isoformat()
        return next((r for r in self.records if r["date"] == iso), None)

    def _own_duty_message(self, rec):
        if not rec or rec.get("date") != date.today().isoformat():
            return None
        if not self.own_duty_message_var.get():
            return None
        own_name = self.own_pharmacy_var.get().strip()
        if not own_name:
            return None
        def pharmacy_key(value):
            return normalize_tr(value).replace("ECZANESI", "").strip()
        own_key = pharmacy_key(own_name)
        roster_names = [pharmacy_key(name) for name in rec["pharmacy"].split("/") if name.strip()]
        if own_key not in roster_names:
            return None
        return f"BUGÜN NÖBETÇİYİZ: {own_name.upper()}"

    def _update_preview(self):
        if not hasattr(self, "_preview"):
            return
        if getattr(self, "free_mode_var", None) and self.free_mode_var.get():
            self.preview_text = self.free_text_var.get().strip() or "Serbest metni yazın"
            self.preview_offset = 0
            self.preview_phase = 0
            self._preview.configure(bg=self.settings.get("background_color", "#071a35"))
            self.preview_label.configure(text=self.preview_text if len(self.preview_text) < 58 else self.preview_text[:55] + "…")
            return
        rec = None
        if hasattr(self, "date_var"):
            try:
                iso = parse_date(self.date_var.get()).isoformat()
                form_name = self.pharmacy_var.get().strip()
                rec = canonical_record({"date": iso, "pharmacy": form_name}) if form_name else next((r for r in self.records if r["date"] == iso), None)
            except ValueError:
                rec = None
        self.preview_text = (self._own_duty_message(rec) or format_display(rec)) if rec else "Tarih ve eczane adlarını girin"
        self.preview_offset = 0
        self.preview_phase = 0
        self._preview.configure(bg=self.settings.get("background_color", "#071a35"))
        self.preview_label.configure(text=self.preview_text if len(self.preview_text) < 58 else self.preview_text[:55] + "…")

    def _preview_tick(self):
        self._preview.delete("all")
        bg = self.settings.get("background_color", "#071a35")
        fg = self.settings.get("text_color", "#ffffff")
        self._preview.configure(bg=bg)
        width = max(300, self._preview.winfo_width())
        text = getattr(self, "preview_text", "Bugünün nöbetçi eczanesi")
        anim = self.animation_var.get() if hasattr(self, "animation_var") else "Sola kaydır"
        font = ("Segoe UI", 12, "bold")
        measure_font = tkfont.Font(family="Segoe UI", size=12, weight="bold")
        self.preview_phase += 1
        if anim == "Sabit":
            self._preview.create_text(width / 2, 55, text=text, fill=fg, font=font, anchor="center")
        elif anim in ("Sola kaydır", "Sağa kaydır"):
            text_width = max(160, measure_font.measure(text))
            cycle = text_width + 70
            if anim == "Sola kaydır":
                x = width - self.preview_offset
                second_x = x + cycle
                self.preview_offset = (self.preview_offset + max(1, self.speed_var.get() // 22)) % cycle
            else:
                x = -text_width + self.preview_offset
                second_x = x - cycle
                self.preview_offset = (self.preview_offset + max(1, self.speed_var.get() // 22)) % cycle
            self._preview.create_text(x, 55, text=text, fill=fg, font=font, anchor="w")
            self._preview.create_text(second_x, 55, text=text, fill=fg, font=font, anchor="w")
        elif anim == "Yanıp sön":
            if (self.preview_phase // 8) % 2 == 0:
                self._preview.create_text(width / 2, 55, text=text, fill=fg, font=font, anchor="center")
        elif anim == "Nefes efekti":
            level = 0.2 + 0.8 * (0.5 + 0.5 * math.sin(self.preview_phase / max(3, 18 - self.speed_var.get() // 7)))
            try:
                rgb = tuple(int(fg[i:i + 2], 16) for i in (1, 3, 5))
                faded = "#" + "".join(f"{int(channel * level):02x}" for channel in rgb)
            except (ValueError, TypeError):
                faded = fg
            self._preview.create_text(width / 2, 55, text=text, fill=faded, font=font, anchor="center")
        else:
            text_width = max(160, measure_font.measure(text))
            cycle = text_width + 70
            x = width - self.preview_offset
            y = 38 + (self.preview_phase * max(1, self.speed_var.get() // 25)) % 34
            self._preview.create_text(x, y, text=text, fill=fg, font=font, anchor="w")
            self._preview.create_text(x + cycle, y, text=text, fill=fg, font=font, anchor="w")
            self.preview_offset = (self.preview_offset + max(1, self.speed_var.get() // 22)) % cycle
        self._preview.after(55, self._preview_tick)

    def send_today(self):
        rec = self._selected_record(for_today=True)
        if not rec:
            messagebox.showinfo(APP_TITLE, "Bugün için nöbetçi eczane kaydı bulunamadı.")
            return
        self._send_record(rec, "Bugünün kaydı panele gönderiliyor…")

    def send_preview(self):
        if self.free_mode_var.get():
            self.send_free_text()
            return
        rec = self._selected_record()
        if not rec:
            messagebox.showinfo(APP_TITLE, "Seçili tarih için önce bir eczane kaydı girin.")
            return
        self._send_record(rec, f"{rec['date']} kaydı panele gönderiliyor…")

    def _free_mode_changed(self):
        self._save_control_settings()

    def send_free_text(self):
        text = self.free_text_var.get().strip()
        if not text:
            messagebox.showinfo(APP_TITLE, "Önce serbest metin alanını doldurun.")
            return
        if len(text) > 500:
            messagebox.showerror(APP_TITLE, "Gönderilecek metin 500 karakteri aşıyor.")
            return
        self._save_control_settings()
        options = self._send_options()
        def completed(result):
            self._remember_slot(text, options, result)
            self._set_status("Serbest metin panele gönderildi.")
        self._background(self._send_async(text, options), "Serbest metin panele gönderiliyor…", completed)

    def _send_options(self):
        return {
            "device_name": self._current_device_name(),
            "brightness": self.brightness_var.get(),
            "animation": ANIMATIONS.get(self.animation_var.get(), 1),
            "speed": self.speed_var.get(),
            "color": self.settings.get("text_color", "#ffffff").lstrip("#"),
            "bg_color": self.settings.get("background_color", "#071a35").lstrip("#"),
            "font": self.font_var.get() or "UNIFONT",
            "save_slot": int(self.save_slot_var.get()) if self.save_to_slot_var.get() else 0,
            "orientation": self._current_orientation(),
            "mirror_horizontal": self.mirror_horizontal_var.get(),
            "mirror_vertical": self.mirror_vertical_var.get(),
        }

    def _send_record(self, rec, message, on_success=None):
        self._save_control_settings()
        text = self._own_duty_message(rec) or format_display(rec)
        if len(text) > 500:
            messagebox.showerror(APP_TITLE, "Gönderilecek metin 500 karakteri aşıyor. Eczane adlarını kısaltın.")
            return
        def completed(result):
            self._remember_slot(text, options, result)
            self._set_status(f"Gönderim tamamlandı: {rec['pharmacy']} — {rec['date']}")
            if on_success:
                on_success(result)
        options = self._send_options()
        self._background(self._send_async(text, options), message, completed)

    def _remember_slot(self, text, options, result):
        slot = int(options.get("save_slot", 0))
        if slot > 0:
            self.slot_manifest[str(slot)] = {"text": text, "updated": datetime.now().strftime("%d.%m.%Y %H:%M"), "device": result.get("device", "") if isinstance(result, dict) else ""}
            self._save()
            self._refresh_slots()

    async def _send_async(self, text, options):
        from bleak import BleakScanner
        from pypixelcolor import AsyncClient
        devices = await BleakScanner.discover(timeout=7)
        compatible = [d for d in devices if d.name and d.name.startswith(DEVICE_PREFIX)]
        preferred = options.get("device_name", "")
        match = next((d for d in compatible if preferred and d.name == preferred), None) or (compatible[0] if compatible else None)
        if not match:
            raise RuntimeError(f"{DEVICE_PREFIX} ile başlayan uyumlu panel bulunamadı. Paneli açın ve telefon bağlantısını kapatın.")
        async with AsyncClient(match.address) as client:
            await client.set_brightness(options["brightness"])
            await client.set_orientation(options.get("orientation", 0))
            save_slot = options.get("save_slot", 0)
            if options.get("mirror_horizontal") or options.get("mirror_vertical"):
                info = client.get_device_info()
                path = make_mirrored_text_gif(text, options, info.width, info.height)
                try:
                    await client.send_image(path, resize_method="fit", save_slot=save_slot)
                    if save_slot > 0:
                        # Keep the current live display outside the device playlist.
                        await client.send_image(path, resize_method="fit", save_slot=0)
                finally:
                    Path(path).unlink(missing_ok=True)
            else:
                await client.send_text(text, save_slot=save_slot, animation=options["animation"], speed=options["speed"], color=options["color"], bg_color=options["bg_color"], font=options["font"])
                if save_slot > 0:
                    await client.send_text(text, save_slot=0, animation=options["animation"], speed=options["speed"], color=options["color"], bg_color=options["bg_color"], font=options["font"])
        return {"device": match.name, "save_slot": save_slot, "text": text}

    def _background(self, coro, message, on_success=None):
        if self.busy:
            if hasattr(coro, "close"):
                coro.close()
            return
        self.busy = True
        self._set_status(message)

        def run():
            try:
                result = asyncio.run(coro)
                self.status_queue.put((True, result, on_success))
            except Exception as exc:
                self.status_queue.put((False, exc, None))
        threading.Thread(target=run, daemon=True).start()

    def _poll_status(self):
        try:
            while True:
                success, result, callback = self.status_queue.get_nowait()
                self.busy = False
                if success:
                    if callback:
                        callback(result)
                else:
                    self._set_status(f"İşlem başarısız: {result}")
                    messagebox.showerror(APP_TITLE, f"İşlem başarısız.\n\n{result}")
        except queue.Empty:
            pass
        self.root.after(120, self._poll_status)

    def _set_status(self, message):
        self.status_var.set(message)

    def _check_daily_send(self):
        if self.auto_var.get() and not self.busy and not self._auto_scan_busy:
            self._auto_scan_busy = True
            self._background(self._auto_scan_async(), "Panel bağlantısı ve günlük gönderim kontrol ediliyor…", self._auto_scan_done)
        self.root.after(20_000, self._check_daily_send)

    async def _auto_scan_async(self):
        from bleak import BleakScanner
        try:
            found = await BleakScanner.discover(timeout=5)
            return [(d.name or "", d.address) for d in found if d.name and d.name.startswith(DEVICE_PREFIX)]
        except Exception:
            return []

    def _auto_scan_done(self, devices):
        self._auto_scan_busy = False
        selected = self._current_device_name()
        match = next((item for item in devices if selected and item[0] == selected), None) or (devices[0] if devices else None)
        was_online = self._auto_device_online
        self._auto_device_online = bool(match)
        if not match:
            self._auto_reconnect_pending = True
            if was_online:
                self._set_status("Panel bağlantısı kesildi; Bluetooth'ta yeniden görünmesi bekleniyor.")
            return
        if not was_online:
            self._auto_reconnect_pending = True
            if not selected:
                self.device_var.set(match[0])
                self.settings["device_name"] = match[0]
                self._save()
        today = date.today().isoformat()
        if self._auto_reconnect_pending or self.settings.get("last_auto_date") != today:
            rec = self._selected_record(for_today=True)
            if rec:
                def mark_sent(_result):
                    self.settings["last_auto_date"] = today
                    self._auto_reconnect_pending = False
                    self._save()
                self._send_record(rec, "Panel hazır; bugünün nöbeti otomatik gönderiliyor…", mark_sent)
            else:
                self._set_status("Bugün için nöbet kaydı yok; panel bulundu.")

    def _close(self):
        self._save_control_settings()
        self.root.destroy()


async def send_today_once():
    saved = json.loads(DATA_PATH.read_text(encoding="utf-8"))
    today = date.today().isoformat()
    record = next((canonical_record(r) for r in saved.get("records", []) if parse_date(r["date"]).isoformat() == today), None)
    if not record:
        raise RuntimeError(f"{today} için nöbet kaydı bulunamadı.")
    settings = {
        "device_name": "",
        "brightness": 70,
        "animation": "Sola kaydır",
        "speed": 70,
        "text_color": "#ffffff",
        "background_color": "#071a35",
        "font": "UNIFONT",
    }
    settings.update(saved.get("settings", {}))
    animation = settings.get("animation", "Sola kaydır")
    options = {
        "device_name": settings.get("device_name", ""),
        "brightness": int(settings.get("brightness", 70)),
        "animation": ANIMATIONS.get(animation, 1),
        "speed": int(settings.get("speed", 70)),
        "color": str(settings.get("text_color", "#ffffff")).lstrip("#"),
        "bg_color": str(settings.get("background_color", "#071a35")).lstrip("#"),
        "font": settings.get("font", "UNIFONT"),
        "save_slot": int(settings.get("save_slot", 1)) if settings.get("save_to_slot", True) else 0,
        "orientation": int(settings.get("orientation", 0)),
        "mirror_horizontal": bool(settings.get("mirror_horizontal", False)),
        "mirror_vertical": bool(settings.get("mirror_vertical", False)),
    }
    text = format_display(record)
    from bleak import BleakScanner
    from pypixelcolor import AsyncClient
    devices = await BleakScanner.discover(timeout=8)
    compatible = [d for d in devices if d.name and d.name.startswith(DEVICE_PREFIX)]
    match = next((d for d in compatible if options["device_name"] and d.name == options["device_name"]), None) or (compatible[0] if compatible else None)
    if not match:
        raise RuntimeError(f"{DEVICE_PREFIX} ile başlayan uyumlu panel bulunamadı. Panel açık ve telefon bağlantısı kapalı olmalı.")
    async with AsyncClient(match.address) as client:
        await client.set_brightness(options["brightness"])
        await client.set_orientation(options["orientation"])
        if options["mirror_horizontal"] or options["mirror_vertical"]:
            info = client.get_device_info()
            image_path = make_mirrored_text_gif(text, options, info.width, info.height)
            try:
                await client.send_image(image_path, resize_method="fit", save_slot=options["save_slot"])
                if options["save_slot"] > 0:
                    await client.send_image(image_path, resize_method="fit", save_slot=0)
            finally:
                Path(image_path).unlink(missing_ok=True)
        else:
            await client.send_text(text, save_slot=options["save_slot"], animation=options["animation"], speed=options["speed"], color=options["color"], bg_color=options["bg_color"], font=options["font"])
            if options["save_slot"] > 0:
                await client.send_text(text, save_slot=0, animation=options["animation"], speed=options["speed"], color=options["color"], bg_color=options["bg_color"], font=options["font"])
    if options["save_slot"] > 0:
        saved.setdefault("slots", {})[str(options["save_slot"])] = {"text": text, "updated": datetime.now().strftime("%d.%m.%Y %H:%M"), "device": match.name}
        DATA_PATH.write_text(json.dumps(saved, ensure_ascii=False, indent=2), encoding="utf-8")
    return {"date": today, "pharmacy": record["pharmacy"], "device": match.name, "save_slot": options["save_slot"], "text": text, "sent_at": datetime.now().astimezone().isoformat(), "result": "sent"}


def main():
    if "--send-today" in sys.argv:
        result_path = LOCAL_APP_DATA / "last_send.json"
        try:
            result = asyncio.run(send_today_once())
            result_path.write_text(json.dumps(result, ensure_ascii=False, indent=2), encoding="utf-8")
            if "--quiet" not in sys.argv:
                root = tk.Tk(); root.withdraw(); messagebox.showinfo(APP_TITLE, f"Bugünün nöbeti panele gönderildi.\n\n{result['text']}"); root.destroy()
            return 0
        except Exception as exc:
            result = {"sent_at": datetime.now().astimezone().isoformat(), "result": "error", "error": str(exc)}
            result_path.write_text(json.dumps(result, ensure_ascii=False, indent=2), encoding="utf-8")
            if "--quiet" not in sys.argv:
                root = tk.Tk(); root.withdraw(); messagebox.showerror(APP_TITLE, f"Bugünün nöbeti gönderilemedi.\n\n{exc}"); root.destroy()
            return 1
    root = tk.Tk()
    PharmacyPanelApp(root)
    root.mainloop()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

