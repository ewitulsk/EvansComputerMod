"""Card Designer (dev.ecm.carddesigner): draw a card's two faces, name it,
write its stripe and print it (docs/cards/CARDS.md "Programs and apps").

Front / Back tabs over a pixel_grid (the chassis' fixed features are masked:
the chip pad on the front, the stripe rows on the back), the tools pencil /
fill / eyedropper / line and a mirror-H toggle, a 16-swatch palette (click a
swatch to paint with it; the colour picker edits it), the printed fields
(name, number, issuer, label), the stripe tracks (bank / hybrid), New (choose
the kind), Open / Save `.card` files under /var/apps/dev.ecm.carddesigner/,
Print on an attached Card Printer (its dye cost first, then the printer's
verdict) and Export (the .card file for `cardtool print` elsewhere) when no
printer is attached. The app is a printer client only: it never touches a
card's chip or sectors.

The design lives in a cards.Design; the grids are views of its faces.
"""

import ui
import cards
import peripheral

APP_ID = "dev.ecm.carddesigner"
DATA = "/var/apps/" + APP_ID

# A friendly starting palette: index 0 is the card's background.
DEFAULT_PALETTE = [
    (255, 255, 255), (0, 0, 0), (200, 30, 30), (30, 120, 220),
    (40, 160, 70), (240, 200, 40), (240, 130, 30), (140, 60, 180),
    (90, 90, 90), (180, 180, 180), (120, 70, 30), (250, 170, 190),
    (30, 180, 190), (20, 60, 120), (160, 20, 60), (230, 230, 200),
]

TOOLS = ("pencil", "fill", "eyedropper", "line")
FIELD_KEYS = ("name", "number", "issuer", "label")


def rgb_int(c):
    """[r, g, b] -> 0xRRGGBB."""
    r, g, b = (int(v) & 255 for v in c)
    return (r << 16) | (g << 8) | b


def int_rgb(v):
    v = int(v) & 0xFFFFFF
    return [v >> 16, (v >> 8) & 255, v & 255]


def has_stripe(kind):
    return kind in ("bank", "hybrid")


def chip_pad(kind):
    """The contact pad rectangle (x0, x1, y0, y1 inclusive) on the front, or None."""
    if kind in ("bank", "hybrid"):
        return (3, 8, 7, 11)
    if kind == "sim":
        return (1, 6, 3, 7)
    return None


def face_mask(kind, face):
    """The pixel_grid mask of a face: 1 where the chassis draws a fixed feature
    (the renderer paints over those texels, so the Designer locks them)."""
    w, h = cards.face_size(kind)
    mask = [0] * (w * h)
    if face == "front":
        pad = chip_pad(kind)
        if pad:
            x0, x1, y0, y1 = pad
            for y in range(y0, y1 + 1):
                for x in range(x0, x1 + 1):
                    mask[y * w + x] = 1
    elif has_stripe(kind):
        for y in range(2, 6):
            for x in range(w):
                mask[y * w + x] = 1
    return mask


def full_palette(palette):
    """A design's palette padded to 16 entries (the swatches) as [r, g, b] lists."""
    out = [list(c) for c in palette][:16]
    i = len(out)
    while len(out) < 16:
        out.append(list(DEFAULT_PALETTE[i]))
        i += 1
    return out


def face_pixels(design, face):
    w, h = design.width, design.height
    return [design.pixel(face, x, y) for y in range(h) for x in range(w)]


def card_files(data_dir):
    """The .card files under the app's data folder (names without the folder)."""
    names = []
    try:
        import shell
        listing = shell.list_files() or ""
    except Exception:
        listing = ""
    prefix = data_dir.strip("/") + "/"
    for line in listing.split("\n"):
        p = line.strip().lstrip("/")
        if p.startswith(prefix) and p.endswith(".card") and "/" not in p[len(prefix):]:
            names.append(p[len(prefix):])
    names.sort()
    return names


class Designer:
    """The app: build it, then run()."""

    def __init__(self, app=None, data_dir=DATA):
        self.app = app or ui.App(APP_ID)
        self.data_dir = data_dir
        self.design = cards.Design.new("bank")
        self.design.palette = full_palette(self.design.palette)
        self.index = 1
        self.tool = "pencil"
        self.mirror = False
        self.printing = None
        self.dialog = None
        self._build()
        self.status("New bank card. Draw, then Save or Print.")

    # ------------------------------------------------------------ widgets

    def _build(self):
        d = self.design
        self.grid_front = self._grid("front")
        self.grid_back = self._grid("back")
        self.tabs = ui.Tabs(["Front", "Back"], [self.grid_front, self.grid_back], selected=0, id="faces")
        self.tool_buttons = {}
        tools = []
        for t in TOOLS:
            b = ui.Button(t.capitalize(), id="tool_" + t, on_click=self._tool_handler(t))
            self.tool_buttons[t] = b
            tools.append(b)
        self.mirror_toggle = ui.Toggle("Mirror", checked=False, id="mirror", on_change=self._on_mirror)
        self.swatches = ui.PixelGrid(width=8, height=2, pixels=list(range(16)), palette=self._palette_ints(),
                                     zoom=14, tool="eyedropper", grid=True, id="swatches",
                                     on_select=self._on_swatch)
        self.picker = ui.ColorPicker(value=rgb_int(d.palette[self.index]), id="picker", on_change=self._on_color)
        self.index_label = ui.Label("Colour 1", style="caption", id="index")
        self.fields = {}
        field_row = []
        for k in FIELD_KEYS:
            f = ui.TextField(text=d.fields.get(k, ""), placeholder=k, id="field_" + k, grow=1,
                             on_change=self._field_handler(k))
            self.fields[k] = f
            field_row.append(f)
        self.tracks = []
        track_row = []
        for i in range(3):
            f = ui.TextField(placeholder="stripe track %d" % (i + 1), id="track%d" % (i + 1), grow=1,
                             on_change=self._track_handler(i))
            self.tracks.append(f)
            track_row.append(f)
        self.filename = ui.TextField(text="my.card", placeholder="file name", id="filename", width=140)
        self.status_label = ui.Label("", id="status", wrap=True)
        self.print_button = ui.Button("Print", id="print", style="primary", on_click=lambda e: self.print_card())
        self.export_button = ui.Button("Export", id="export", on_click=lambda e: self.export())
        self.tools_row = ui.Row(gap=4, children=tools + [self.mirror_toggle])
        self.side = ui.Column(gap=6, children=[
            self.tools_row,
            ui.Label("Palette", style="caption"),
            self.swatches,
            self.index_label,
            self.picker,
        ])
        self.stripe_row = ui.Row(gap=4, children=track_row)
        self.body = ui.Column(gap=6, padding=8, children=[
            ui.Row(gap=4, children=[
                self.filename,
                ui.Button("New", id="new", on_click=lambda e: self.ask_kind()),
                ui.Button("Open", id="open", on_click=lambda e: self.ask_open()),
                ui.Button("Save", id="save", on_click=lambda e: self.save()),
                self.print_button,
                self.export_button,
            ]),
            self.status_label,
            ui.Row(gap=10, children=[self.tabs, self.side]),
            ui.Label("Fields", style="caption"),
            ui.Row(gap=4, children=field_row),
            ui.Label("Stripe", style="caption", id="stripe_caption"),
            self.stripe_row,
        ])
        self.stack = ui.Stack(children=[self.body])
        self.win = self.app.window("Card Designer", self.stack)
        self._apply_kind()
        self._mark_tool()

    def _grid(self, face):
        d = self.design
        w, h = d.width, d.height
        zoom = 8 if w >= 20 else 16
        return ui.PixelGrid(width=w, height=h, pixels=face_pixels(d, face), palette=self._palette_ints(),
                            zoom=zoom, color=self.index, tool=self.tool, mirror_h=self.mirror, grid=True,
                            mask=face_mask(d.kind, face), id="grid_" + face,
                            on_change=self._change_handler(face), on_select=self._on_pick)

    def _palette_ints(self):
        return [rgb_int(c) for c in self.design.palette]

    def _grids(self):
        return (self.grid_front, self.grid_back)

    # ------------------------------------------------------------ handlers

    def _tool_handler(self, tool):
        def on_click(_=None):
            self.set_tool(tool)
        return on_click

    def set_tool(self, tool):
        if tool not in TOOLS:
            raise ValueError("tool is one of %s" % (TOOLS,))
        self.tool = tool
        for g in self._grids():
            g.tool = tool
        self._mark_tool()

    def _mark_tool(self):
        for t, b in self.tool_buttons.items():
            b.style = "primary" if t == self.tool else None

    def _on_mirror(self, ev):
        self.set_mirror(bool(ev.value))

    def set_mirror(self, on):
        self.mirror = bool(on)
        self.mirror_toggle.checked = self.mirror
        for g in self._grids():
            g.mirror_h = self.mirror

    def _on_swatch(self, ev):
        self.select_index(int(ev.value))

    def _on_pick(self, ev):
        # The eyedropper on a face: paint with what it found.
        self.select_index(int(ev.value))

    def select_index(self, i):
        self.index = int(i) & 15
        for g in self._grids():
            g.color = self.index
        self.swatches.color = self.index
        self.index_label.text = "Colour %d  #%06x" % (self.index, rgb_int(self.design.palette[self.index]))
        self.picker.value = rgb_int(self.design.palette[self.index])

    def _on_color(self, ev):
        self.set_color(int(ev.value))

    def set_color(self, value):
        """Give the selected swatch a new colour (0xRRGGBB)."""
        self.design.palette[self.index] = int_rgb(value)
        pal = self._palette_ints()
        for g in self._grids():
            g.palette = pal
        self.swatches.palette = pal
        self.index_label.text = "Colour %d  #%06x" % (self.index, int(value) & 0xFFFFFF)

    def _change_handler(self, face):
        def on_change(ev):
            for x, y, i in ev.value:
                self.design.set_pixel(face, x, y, i)
        return on_change

    def _field_handler(self, key):
        def on_change(ev):
            self.set_field(key, ev.value)
        return on_change

    def set_field(self, key, value):
        value = str(value)
        if value:
            self.design.fields[key] = value[:64]
        else:
            self.design.fields.pop(key, None)

    def _track_handler(self, i):
        def on_change(ev):
            self.set_track(i, ev.value)
        return on_change

    def set_track(self, i, value):
        t = list(self.design.stripe or ("", "", ""))
        t[i] = str(value)
        self.design.stripe = tuple(t) if any(t) else None

    # ------------------------------------------------------------ the design

    def _apply_kind(self):
        """Show / hide what the kind has (the stripe editor) and refresh the grids."""
        d = self.design
        stripe = has_stripe(d.kind)
        for f in self.tracks:
            f.enabled = stripe
        if not stripe:
            d.stripe = None
        t = list(d.stripe or ("", "", ""))
        for i, f in enumerate(self.tracks):
            f.text = t[i]
        for k, f in self.fields.items():
            f.text = d.fields.get(k, "")

    def set_design(self, design):
        """Replace the design (New / Open): new grids of its size, its palette and texts."""
        design.palette = full_palette(design.palette)
        self.design = design
        self.index = min(self.index, 15)
        self.grid_front = self._grid("front")
        self.grid_back = self._grid("back")
        self.tabs.pages = [self.grid_front, self.grid_back]
        self.tabs.selected = 0
        pal = self._palette_ints()
        self.swatches.palette = pal
        self._apply_kind()
        self.select_index(self.index)

    def new_card(self, kind):
        self.close_dialog()
        d = cards.Design.new(kind)
        self.set_design(d)
        self.status("New %s card (%dx%d)." % (kind, d.width, d.height))

    def path_of(self, name=None):
        name = (name or self.filename.text or "my.card").strip()
        if not name.endswith(".card"):
            name += ".card"
        name = name.replace("/", "_")
        return self.data_dir + "/" + name

    def save(self, name=None):
        path = self.path_of(name)
        try:
            self.design.save(path)
        except Exception as e:
            self.status("Cannot save %s: %s" % (path, e))
            return None
        self.status("Saved %s" % path)
        return path

    def load(self, name):
        self.close_dialog()
        path = self.path_of(name)
        try:
            d = cards.Design.load(path)
        except Exception as e:
            self.status("Cannot open %s: %s" % (path, e))
            return False
        self.filename.text = path[len(self.data_dir) + 1:]
        self.set_design(d)
        self.status("Opened %s (%s, %d colours)." % (path, d.kind, len([c for c in d.palette])))
        return True

    def export(self):
        """The .card file for a computer that has a printer (`cardtool print <file>`)."""
        path = self.save()
        if path:
            self.status("Exported %s: copy it to a computer with a Card Printer and run  cardtool print %s" % (path, path))

    # ------------------------------------------------------------ printing

    def printer(self):
        try:
            return cards.Printer.find()
        except Exception:
            return None

    def print_card(self):
        p = self.printer()
        if p is None:
            self.status("No Card Printer is attached: Export the .card file instead.")
            return False
        try:
            if not p.present():
                self.status("Put a blank %s card in the printer first." % self.design.kind)
                return False
            cost = p.cost(self.design)
            dyes = cost.get("dyes", cost) if isinstance(cost, dict) else cost
            r = p.print(self.design)
        except peripheral.PeripheralError as e:
            self.status("Printer %s refused: %s" % (p.name, e))
            return False
        self.printing = p.name
        self.print_button.enabled = False
        self.status("Printing on %s (dyes: %s)..." % (p.name, self._dyes_text(dyes)))
        return True

    @staticmethod
    def _dyes_text(dyes):
        if isinstance(dyes, dict):
            return ", ".join("%s x%s" % (k, v) for k, v in sorted(dyes.items())) or "none"
        return str(dyes)

    def poll_printer(self):
        """Pick up the printer's verdict (print_done / print_failed) without blocking."""
        if not self.printing:
            return
        for _ in range(8):
            ev = peripheral.pull_event(None, timeout=0)
            if not ev:
                break
            if len(ev) < 2 or ev[1] != self.printing:
                continue
            if ev[0] == "print_done":
                self.status("Printed on %s." % self.printing)
                self._print_over()
            elif ev[0] == "print_failed":
                self.status("Print failed on %s: %s" % (self.printing, ev[2] if len(ev) > 2 else "?"))
                self._print_over()

    def _print_over(self):
        self.printing = None
        self.print_button.enabled = True

    # ------------------------------------------------------------ dialogs

    def ask_kind(self):
        buttons = [ui.Button(k, id="kind_" + k, on_click=self._kind_handler(k)) for k in cards.KINDS]
        self.show_dialog(ui.Dialog("New card", children=[
            ui.Label("Which kind of card?"),
            ui.Row(gap=4, children=buttons),
            ui.Button("Cancel", id="cancel", on_click=lambda e: self.close_dialog()),
        ]))

    def _kind_handler(self, kind):
        return lambda e: self.new_card(kind)

    def ask_open(self):
        names = card_files(self.data_dir)
        if not names:
            self.status("No .card files in %s yet: Save one first." % self.data_dir)
            return
        lst = ui.List(items=names, id="files", on_select=lambda e: self.load(names[int(e.value)]))
        self.show_dialog(ui.Dialog("Open", children=[
            lst,
            ui.Button("Cancel", id="cancel", on_click=lambda e: self.close_dialog()),
        ]))

    def show_dialog(self, dialog):
        self.dialog = dialog
        self.stack.children = [self.body, dialog]

    def close_dialog(self):
        self.dialog = None
        self.stack.children = [self.body]

    def status(self, text):
        self.status_label.text = str(text)

    # ------------------------------------------------------------ loop

    def step(self, timeout_ms=-1):
        if self.printing:
            timeout_ms = 200
        alive = self.app.step(timeout_ms)
        self.poll_printer()
        return alive

    def run(self):
        while self.step(-1):
            pass


def main():
    Designer().run()


if not globals().get("_TESTING"):
    main()
