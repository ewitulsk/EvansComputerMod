"""Graphical apps: windows, widgets and layout (docs/gui/UI_PROTOCOL.md §13).

    import ui

    app = ui.App("dev.evan.notes")
    notes = app.restore().get("notes", [])
    entry = ui.TextField(placeholder="New note")
    rows = ui.List(items=[ui.Label(n) for n in notes])

    def add(_=None):
        if entry.text:
            notes.append(entry.text)
            entry.text = ""
            rows.items = [ui.Label(n) for n in notes]

    app.window("Notes", ui.Column(gap=8, padding=12, children=[
        ui.Row(gap=8, children=[entry, ui.Button("Add", on_click=add)]),
        rows,
    ]))
    app.hotkey(1, "Add note", add)
    app.on_suspend(lambda: {"notes": notes})
    app.run()

Widgets keep their own values: change them (entry.text = "", box.checked
= True, rows.items = [...]) and the window updates on the next loop turn.
Layout, text wrapping, focus and drawing run in the same Rust toolkit Rust
apps use, so both look and behave the same.

Widgets: Label, Button, TextField, TextArea, List, ScrollView, Checkbox,
Toggle, Slider, Tabs, Menu, Dialog, Image, Canvas, PixelGrid (an editable
indexed bitmap: card faces, icons) and ColorPicker. Layout: Row, Column,
Stack, Grid, Spacer, Responsive (one layout per size class: compact / medium
/ wide).
Common options: id, width, height, grow, align ("start", "center", "end",
"stretch"), padding, gap, enabled, style ("primary", "danger" for buttons;
"title", "caption" for labels), font ("ui" / "mono"), size, color.
Handlers receive an Event with .kind, .value and .widget.

ui.load("app.ui.json", handlers={"add_note": add}) opens a window from a
Visual Programmer layout file.

Pixels: app.canvas(w, h, format="rgba8888") makes a Canvas widget with its
own pixels. canvas.put(pixels, damage=None) shows new pixels at the next loop
turn; canvas.commit_at(pts_us) shows the ones put since the last commit at a
time on ui.now_us()'s clock instead (smooth video: queue frames a little
ahead), canvas.queue_flush() drops the queued ones (seek, pause) and
canvas.content = "motion" lets them travel lossy ("auto", "exact").
"""

import _ui

_COMMON = ("width", "height", "grow", "align", "padding", "gap", "enabled", "style",
           "font", "size", "color", "wrap")


class Event:
    """A widget event: kind is "click", "change", "submit" or "select"."""

    def __init__(self, kind, value, widget):
        self.kind = kind
        self.value = value
        self.widget = widget

    def __repr__(self):
        return "Event(%r, %r)" % (self.kind, self.value)


class Widget:
    _kind = None
    _props = ()
    _events = ()

    def __init__(self, **kw):
        self.id = kw.pop("id", None)
        self._opts = {}
        self._handlers = {}
        for k, v in kw.items():
            if k.startswith("on_") and k[3:] in self._events:
                self._handlers[k[3:]] = v
            elif k in _COMMON or k in self._props:
                setattr(self, k, v)
            else:
                raise TypeError("%s got an unexpected option %r" % (type(self).__name__, k))

    def __setattr__(self, k, v):
        if k in _COMMON:
            self.__dict__.setdefault("_opts", {})[k] = v
        else:
            object.__setattr__(self, k, v)

    def __getattr__(self, k):
        if k in _COMMON:
            return self.__dict__.get("_opts", {}).get(k)
        raise AttributeError(k)

    def on(self, kind, fn):
        """Set the handler for "click", "change", "submit" or "select"."""
        self._handlers[kind] = fn
        return self

    # -- serialisation to the toolkit (the .ui.json form)

    def _fields(self):
        return {}

    def _internal(self, kind):
        """A handler the widget needs for itself (keeping values in sync)."""
        return None

    def _tree(self, refs):
        body = {}
        if self.id is not None:
            body["id"] = self.id
        for k, v in self._opts.items():
            if v is not None:
                body[k] = v
        body.update(self._fields())
        for kind in self._events:
            mine = self._internal(kind)
            user = self._handlers.get(kind)
            if mine is None and user is None:
                continue
            refs.append((self, mine, user))
            body["on_" + kind] = len(refs) - 1
        kids = self._kids()
        if kids is not None:
            body["children"] = [c._tree(refs) for c in kids]
        return {self._kind: body}

    def _kids(self):
        return None

    def _repush(self):
        """Send the values again at the next sync (after window(), whose
        serialisation only declared the surface)."""
        if "_push" in self.__dict__:
            self.__dict__["_push"] = True
        for c in self._kids() or ():
            c._repush()


class Label(Widget):
    _kind = "label"
    _props = ("text",)
    _events = ("click",)

    def __init__(self, text="", **kw):
        self.text = str(text)
        Widget.__init__(self, **kw)

    def _fields(self):
        return {"text": str(self.text)}


class Button(Widget):
    _kind = "button"
    _props = ("text",)
    _events = ("click",)

    def __init__(self, text="", **kw):
        self.text = str(text)
        Widget.__init__(self, **kw)

    def _fields(self):
        return {"text": str(self.text)}


class TextField(Widget):
    """A one-line text field. .text follows what the player types."""

    _kind = "textfield"
    _props = ("placeholder", "password")
    _events = ("change", "submit")

    def __init__(self, text="", placeholder="", password=False, **kw):
        self.placeholder = placeholder
        self.password = bool(password)
        self._text = str(text)
        self._push = bool(text)
        Widget.__init__(self, **kw)

    @property
    def text(self):
        return self._text

    @text.setter
    def text(self, value):
        self._text = str(value)
        self._push = True

    def _fields(self):
        f = {}
        if self.placeholder:
            f["placeholder"] = self.placeholder
        if getattr(self, "password", False):
            f["password"] = True
        if self._push:
            f["text"] = self._text
            self._push = False
        return f

    def _internal(self, kind):
        if kind == "change":
            return self._sync
        return None

    def _sync(self, ev):
        self._text = ev.value


class TextArea(TextField):
    """A multi-line text editor (selection, Ctrl+C/X/V, scrolling). .text
    follows the edits; line_numbers=True shows a gutter."""

    _kind = "textarea"
    _props = ("placeholder", "line_numbers")
    _events = ("change",)

    def __init__(self, text="", placeholder="", line_numbers=False, **kw):
        self.line_numbers = bool(line_numbers)
        TextField.__init__(self, text=text, placeholder=placeholder, **kw)

    def _fields(self):
        f = TextField._fields(self)
        if self.line_numbers:
            f["line_numbers"] = True
        return f


class _Toggleish(Widget):
    _props = ("text",)
    _events = ("change",)

    def __init__(self, text="", checked=False, **kw):
        self.text = str(text)
        self._checked = bool(checked)
        self._push = True
        Widget.__init__(self, **kw)

    @property
    def checked(self):
        return self._checked

    @checked.setter
    def checked(self, value):
        self._checked = bool(value)
        self._push = True

    def _fields(self):
        f = {"text": self.text}
        if self._push:
            f["checked"] = self._checked
            self._push = False
        return f

    def _internal(self, kind):
        return self._sync

    def _sync(self, ev):
        self._checked = bool(ev.value)


class Checkbox(_Toggleish):
    _kind = "checkbox"


class Toggle(_Toggleish):
    _kind = "toggle"


class Slider(Widget):
    """Whole-number slider from min to max in steps of step."""

    _kind = "slider"
    _props = ()
    _events = ("change",)

    def __init__(self, min=0, max=100, value=None, step=1, **kw):
        self.min, self.max, self.step = int(min), int(max), int(step)
        self._value = int(min if value is None else value)
        self._push = True
        Widget.__init__(self, **kw)

    @property
    def value(self):
        return self._value

    @value.setter
    def value(self, v):
        self._value = int(v)
        self._push = True

    def _fields(self):
        f = {"min": self.min, "max": self.max, "step": self.step}
        if self._push:
            f["value"] = self._value
            self._push = False
        return f

    def _internal(self, kind):
        return self._sync

    def _sync(self, ev):
        self._value = ev.value


class _Selectable(Widget):
    _events = ("select",)

    def _init_sel(self, selected):
        self._selected = selected
        self._push = selected is not None

    @property
    def selected(self):
        return self._selected

    @selected.setter
    def selected(self, i):
        self._selected = i
        self._push = True

    def _sel_fields(self, f):
        if self._push:
            f["selected"] = self._selected
            self._push = False
        return f

    def _internal(self, kind):
        return self._sync

    def _sync(self, ev):
        self._selected = ev.value


class List(_Selectable):
    """Rows of widgets (or strings); .selected is the chosen row."""

    _kind = "list"
    _props = ("items",)

    def __init__(self, items=(), selected=None, **kw):
        self.items = list(items)
        self._init_sel(selected)
        Widget.__init__(self, **kw)

    def _kids(self):
        return [i if isinstance(i, Widget) else Label(str(i)) for i in self.items]

    def _fields(self):
        return self._sel_fields({})


class Menu(_Selectable):
    _kind = "menu"
    _props = ("items",)

    def __init__(self, items=(), selected=None, **kw):
        self.items = [str(i) for i in items]
        self._init_sel(selected)
        Widget.__init__(self, **kw)

    def _fields(self):
        return self._sel_fields({"items": list(self.items)})


class Tabs(_Selectable):
    """Tabs(["One", "Two"], [page_one, page_two])."""

    _kind = "tabs"
    _props = ("titles", "pages")

    def __init__(self, titles=(), pages=(), selected=None, **kw):
        self.titles = [str(t) for t in titles]
        self.pages = list(pages)
        self._init_sel(selected)
        Widget.__init__(self, **kw)

    def _fields(self):
        return self._sel_fields({"items": list(self.titles)})

    def _kids(self):
        return self.pages


class _Container(Widget):
    _props = ("children",)

    def __init__(self, children=(), **kw):
        self.children = list(children)
        Widget.__init__(self, **kw)

    def _kids(self):
        return self.children


class Row(_Container):
    _kind = "row"


class Column(_Container):
    _kind = "column"


class Stack(_Container):
    _kind = "stack"


class Grid(_Container):
    """columns: a number, or (compact, medium, wide)."""

    _kind = "grid"
    _props = ("children", "columns")

    def __init__(self, children=(), columns=2, **kw):
        self.columns = columns
        _Container.__init__(self, children, **kw)

    def _fields(self):
        c = self.columns
        return {"columns": list(c) if isinstance(c, (list, tuple)) else int(c)}


class Spacer(Widget):
    _kind = "spacer"


class ScrollView(Widget):
    _kind = "scroll"
    _props = ("child",)

    def __init__(self, child=None, **kw):
        self.child = child
        Widget.__init__(self, **kw)

    def _kids(self):
        return [self.child] if self.child is not None else []


class Dialog(_Container):
    """A modal card over its area: put it last in a Stack."""

    _kind = "dialog"
    _props = ("children", "title")

    def __init__(self, title="", children=(), **kw):
        self.title = str(title)
        _Container.__init__(self, children, **kw)

    def _fields(self):
        return {"text": self.title}


class Responsive(Widget):
    """One layout per size class (compact < 400 lu wide, medium < 800, wide)."""

    _kind = "responsive"
    _props = ("compact", "medium", "wide")

    def __init__(self, compact=None, medium=None, wide=None, **kw):
        self.compact, self.medium, self.wide = compact, medium, wide
        Widget.__init__(self, **kw)

    def _kids(self):
        first = self.compact or self.medium or self.wide
        c = self.compact or first
        m = self.medium or c
        return [c, m, self.wide or m]


class Image(Widget):
    _kind = "image"
    _props = ("resource",)
    _events = ("click",)

    def __init__(self, resource=0, width=0, height=0, **kw):
        self.resource = int(resource)
        Widget.__init__(self, width=width, height=height, **kw)

    def _fields(self):
        return {"resource": self.resource}


_CONTENT = {"auto": 0, "exact": 1, "motion": 2}


class Canvas(Widget):
    """Shows a canvas resource. Made with app.canvas(w, h), it has its own
    pixels: put(), commit_at(), queue_flush() and the content hint."""
    _kind = "canvas"
    _props = ("resource", "scale")
    _events = ("click",)

    def __init__(self, resource=0, width=0, height=0, scale=1, **kw):
        self.resource, self.scale = int(resource), int(scale)
        object.__setattr__(self, "_app", None)
        object.__setattr__(self, "_content", "auto")
        Widget.__init__(self, width=width, height=height, **kw)

    def _fields(self):
        return {"resource": self.resource, "scale": self.scale}

    def _owner(self):
        if self._app is None:
            raise RuntimeError("this Canvas has no pixels of its own (make it with app.canvas(w, h))")
        return self._app

    def put(self, pixels, damage=None):
        """New pixels: bytes in the canvas format, the whole canvas. damage =
        the changed rects [(x, y, w, h), ...] (None: all). They show at the
        next loop turn, or at commit_at's time."""
        app = self._owner()
        _ui.put_canvas(app._h, self.resource, pixels, None if damage is None else [list(r) for r in damage])

    def commit_at(self, pts_us):
        """Show the pixels put since the last commit at pts_us on ui.now_us()'s
        clock (not at the next loop turn). Frames queue in time order, a
        little ahead (about half a second at most)."""
        _ui.commit_at(self._owner()._h, int(pts_us))

    def queue_flush(self):
        """Drop the frames queued with commit_at that aren't shown yet (seek, pause)."""
        _ui.queue_flush(self._owner()._h)

    @property
    def content(self):
        """How the frames may travel to players: "auto", "exact" (never
        lossy) or "motion" (video: lossy allowed)."""
        return self._content

    @content.setter
    def content(self, value):
        if value not in _CONTENT:
            raise ValueError("content is 'auto', 'exact' or 'motion'")
        object.__setattr__(self, "_content", value)
        _ui.set_content(self._owner()._h, self.resource, _CONTENT[value])


def now_us():
    """The monotonic clock in microseconds: the clock commit_at times are on."""
    return _ui.now_us()


def _hex_texels(values):
    """One hex digit per texel (indices 0-15), the compact wire form."""
    return "".join("%x" % (int(v) & 15) for v in values)


class PixelGrid(Widget):
    """An editable indexed bitmap: width x height texels (not lu), each a
    palette index (up to 16 colours, 0xRRGGBB), drawn zoom lu per texel
    (auto-fit without zoom). The player paints with the pointer using tool
    ("pencil", "fill", "eyedropper" or "line") in palette index .color,
    mirrored left-right with mirror_h; texels whose mask entry is non-zero
    are locked (drawn hatched, never painted). .pixels follows the edits
    (on_change gets the changed texels as (x, y, index) tuples); the
    eyedropper sets .color and fires on_select with the picked index."""

    _kind = "pixel_grid"
    _props = ("palette", "zoom", "tool", "mirror_h", "grid", "mask")
    _events = ("change", "select")

    def __init__(self, width=16, height=16, pixels=None, palette=None, zoom=None, color=0,
                 tool="pencil", mirror_h=False, grid=True, mask=None, **kw):
        object.__setattr__(self, "cols", int(width))
        object.__setattr__(self, "rows", int(height))
        self.palette = None if palette is None else [int(c) for c in palette]
        self.zoom = None if zoom is None else int(zoom)
        self.tool = str(tool)
        self.mirror_h = bool(mirror_h)
        self.grid = bool(grid)
        self.mask = None if mask is None else [int(m) for m in mask]
        self._pixels = []
        self.pixels = pixels or []
        Widget.__init__(self, color=int(color), **kw)

    # width / height are the bitmap's size in texels (the lu size is zoom *
    # texels, or the space the layout gives the grid).
    def __setattr__(self, k, v):
        if k == "width":
            object.__setattr__(self, "cols", int(v))
        elif k == "height":
            object.__setattr__(self, "rows", int(v))
        else:
            Widget.__setattr__(self, k, v)

    def __getattr__(self, k):
        if k == "width":
            return self.__dict__["cols"]
        if k == "height":
            return self.__dict__["rows"]
        return Widget.__getattr__(self, k)

    @property
    def pixels(self):
        """The bitmap as a list of palette indices, row by row (width * height)."""
        return self._pixels

    @pixels.setter
    def pixels(self, values):
        n = self.cols * self.rows
        px = [int(v) & 15 for v in values][:n]
        px.extend([0] * (n - len(px)))
        self._pixels = px
        self._push = True

    def pixel(self, x, y):
        """The palette index at texel (x, y)."""
        return self._pixels[y * self.cols + x]

    def set_pixel(self, x, y, index):
        """Paint texel (x, y) from the program (shown at the next loop turn)."""
        self._pixels[y * self.cols + x] = int(index) & 15
        self._push = True

    def _fields(self):
        f = {"cols": self.cols, "rows": self.rows, "tool": self.tool, "mirror_h": self.mirror_h, "grid": self.grid}
        if self.palette is not None:
            f["palette"] = list(self.palette)
        if self.zoom is not None:
            f["zoom"] = self.zoom
        if self.mask is not None:
            f["mask"] = _hex_texels(self.mask)
        if self._push:
            f["pixels"] = _hex_texels(self._pixels)
            self._push = False
        return f

    def _internal(self, kind):
        return self._sync_change if kind == "change" else self._sync_select

    def _sync_change(self, ev):
        for x, y, i in ev.value:
            self._pixels[y * self.cols + x] = i

    def _sync_select(self, ev):
        self._opts["color"] = ev.value


class ColorPicker(Widget):
    """An RGB colour picker: a hue strip, a saturation/value square and an
    editable #rrggbb field. .value is the colour as 0xRRGGBB and follows
    every drag and valid hex edit (on_change)."""

    _kind = "color_picker"
    _props = ()
    _events = ("change",)

    def __init__(self, value=0, **kw):
        self._value = int(value) & 0xFFFFFF
        self._push = True
        Widget.__init__(self, **kw)

    @property
    def value(self):
        return self._value

    @value.setter
    def value(self, v):
        self._value = int(v) & 0xFFFFFF
        self._push = True

    @property
    def hex(self):
        """The colour as "#rrggbb"."""
        return "#%06x" % self._value

    def _fields(self):
        f = {}
        if self._push:
            f["value"] = self._value
            self._push = False
        return f

    def _internal(self, kind):
        return self._sync

    def _sync(self, ev):
        self._value = ev.value


class _Layout:
    """A window from a .ui.json file: the tree is sent as written. Change a
    widget with set("id", text=...) (the change is drawn at the next loop
    turn) and read what the player did with value("id")."""

    def __init__(self, root, handlers, app=None):
        self.root = root
        self.handlers = handlers
        self._app = app
        self._win = 0

    def _find(self, id):
        """(kind, props) of the widget with this id, or (None, None)."""
        def walk(node):
            if isinstance(node, list):
                for x in node:
                    r = walk(x)
                    if r[1] is not None:
                        return r
            elif isinstance(node, dict) and len(node) == 1:
                kind, props = next(iter(node.items()))
                if isinstance(props, dict):
                    if props.get("id") == id:
                        return kind, props
                    for k, v in props.items():
                        if k in ("children", "child", "compact", "medium", "wide"):
                            r = walk(v)
                            if r[1] is not None:
                                return r
            return None, None
        return walk(self.root)

    def find(self, id):
        """The props dict of the widget with this id (None if there is none)."""
        return self._find(id)[1]

    def get(self, id, prop, default=None):
        node = self.find(id)
        return default if node is None else node.get(prop, default)

    def set(self, id, **props):
        """Change props of the widget with this id, e.g. set("count", text="3")."""
        node = self.find(id)
        if node is None:
            raise KeyError("no widget with id " + repr(id))
        for k, v in props.items():
            if k in ("text", "placeholder") and not isinstance(v, str):
                v = str(v)
            node[k] = v

    def value(self, id):
        """What the widget holds now: a text field's text, a checkbox's or
        toggle's state, a slider's value, a list's / tab's selection, a pixel
        grid's bitmap, a colour picker's 0xRRGGBB."""
        kind, node = self._find(id)
        if node is None:
            return None
        state = _ui.widget_state(self._app._h, self._win, id) if self._app is not None else None
        if kind in ("checkbox", "toggle"):
            return state["checked"] if state else bool(node.get("checked", False))
        if kind == "slider":
            return state["value"] if state else int(node.get("value", node.get("min", 0)))
        if kind in ("list", "tabs", "menu"):
            return state["selected"] if state else node.get("selected")
        if kind == "textfield":
            return state["text"] if state else node.get("text", "")
        if kind == "pixel_grid":
            return state["pixels"] if state else node.get("pixels", [])
        if kind == "color_picker":
            return state["value"] if state else int(node.get("value", 0))
        return node.get("text", "")

    def _repush(self):
        pass

    def _tree(self, refs):
        def walk(node):
            if isinstance(node, dict):
                out = {}
                for k, v in node.items():
                    if k.startswith("on_") and isinstance(v, str):
                        refs.append((None, None, self.handlers.get(v)))
                        out[k] = len(refs) - 1
                    else:
                        out[k] = walk(v)
                return out
            if isinstance(node, list):
                return [walk(x) for x in node]
            return node
        return walk(self.root)


class App:
    """A GUI app. ui.App("dev.you.app") connects to the compositor; it fails
    with RuntimeError when no GUI is running (console mode)."""

    def __init__(self, app_id, shell=False):
        self.app_id = app_id
        self._h = _ui.connect(app_id, 1 if shell else 0)
        self._windows = {}
        self._refs = {}
        self._hotkeys = {}
        self._on_suspend = None
        self._on_frame = None
        self._on_close = None
        self._on_message = None
        self._on_media = None
        self._on_background = None

    def restore(self):
        """The dict saved by on_suspend in the previous run ({} if none)."""
        data = _ui.restore(self._h)
        if not data:
            return {}
        try:
            value = _ui.json_loads(data)
        except ValueError:
            return {}
        return value if isinstance(value, dict) else {}

    def _serialise(self, root):
        refs = []
        tree = root._tree(refs)
        return tree, refs

    def window(self, title, root):
        """Open a window showing root; returns its id."""
        tree, refs = self._serialise(root)
        win = _ui.window(self._h, title, tree)
        # The window is drawn after the first step's sync, which serialises
        # again: keep the widgets' initial values for it.
        root._repush()
        self._windows[win] = root
        self._refs[win] = refs
        return win

    def set_root(self, win, root):
        self._windows[win] = root

    def canvas(self, w, h, format="rgba8888"):
        """A Canvas widget w x h lu with its own w x h pixels in format
        ("rgba8888", "rgb565" or "indexed8"); fill it with put()."""
        res = _ui.canvas(self._h, int(w), int(h), format)
        c = Canvas(resource=res, width=int(w), height=int(h), scale=1)
        object.__setattr__(c, "_app", self)
        return c

    def layout(self, win=None):
        """The layout of a window opened with ui.load (None for other windows)."""
        root = self._windows.get(win or self._first())
        return root if isinstance(root, _Layout) else None

    def close_window(self, win):
        self._windows.pop(win, None)
        self._refs.pop(win, None)
        _ui.close_window(self._h, win)

    def size(self, win=None):
        """(width, height, size class) of a window."""
        return _ui.window_size(self._h, win or self._first())

    def hotkey(self, slot, label, fn):
        """Claim ePhone hotkey slot 1-6; fn() runs when it is pressed."""
        _ui.hotkey(self._h, slot, label)
        self._hotkeys[slot] = fn

    def on_suspend(self, fn):
        """fn() returns a dict to keep; restore() gives it back after a restart."""
        self._on_suspend = fn

    def on_frame(self, fn):
        """fn(time_ms) after request_frame(), at the next refresh while visible."""
        self._on_frame = fn

    def on_close(self, fn):
        """fn(win) when the player closes a window (default: close it; quit after the last)."""
        self._on_close = fn

    def on_message(self, fn):
        """fn(name, json_text) for other messages (clipboard data, picked files, ...)."""
        self._on_message = fn

    def request_frame(self, win=None):
        _ui.request_frame(self._h, win or self._first())

    def notify(self, title, body="", tag="", actions=()):
        _ui.send(self._h, "Notify", {"tag": tag, "title": title, "body": body, "actions": list(actions)})

    def copy(self, text):
        _ui.send(self._h, "ClipboardSet", {"mime": "text/plain;charset=utf-8", "data": text})

    def paste(self):
        """Ask for the clipboard; it arrives as on_message("ClipboardData", ...)."""
        _ui.send(self._h, "ClipboardRequest", {"mime": "text/plain;charset=utf-8"})

    def pick_file(self, save=False, types=(), suggested="", req=1):
        _ui.send(self._h, "PickFile", {"req": req, "mode": "save" if save else "open",
                                       "types": list(types), "suggested": suggested})

    def open(self, path):
        _ui.send(self._h, "Open", {"path": path})

    def launch(self, app_id, args=()):
        _ui.send(self._h, "Launch", {"app_id": app_id, "args": list(args)})

    def background_done(self):
        _ui.send(self._h, "BackgroundDone", {})

    def background_mode(self, mode, on=True):
        """Keep running while hidden for a mode declared in app.toml ("audio", "call",
        "radio", "transfer", "navigation"); on=False stops it. The answer comes to
        on_background_result(fn)."""
        _ui.background_mode(self._h, mode, bool(on))

    def media_session(self, title="", artist="", album="", state="playing", position_ms=0,
                      duration_ms=0, actions=("play", "pause")):
        """What is playing, for the Now Playing card (state "playing", "paused",
        "buffering"; "none" ends it). actions: "play", "pause", "next", "previous",
        "seek"; presses come to on_media(fn)."""
        _ui.media_session(self._h, {"title": title, "artist": artist, "album": album, "state": state,
                                    "position_ms": int(position_ms), "duration_ms": int(duration_ms),
                                    "actions": list(actions)})

    def on_media(self, fn):
        """fn(action, position_ms) for a Now Playing control (play, pause, next, previous, seek)."""
        self._on_media = fn

    def on_background_result(self, fn):
        """fn(mode, granted): whether the app may keep running for a background mode."""
        self._on_background = fn

    def quit(self):
        _ui.quit(self._h)

    def _first(self):
        return min(self._windows) if self._windows else 0

    def _sync(self):
        for win, root in self._windows.items():
            if _ui.window_size(self._h, win)[0] == 0:
                # Not configured yet, so nothing is drawn: the tree given to
                # window() stands and the widgets' pushes wait for the first draw.
                continue
            tree, refs = self._serialise(root)
            _ui.set_root(self._h, win, tree)
            self._refs[win] = refs
        return _ui.render(self._h)

    def _dispatch(self, ev):
        kind = ev[0]
        if kind == "widget":
            _, win, ref, ekind, value = ev
            refs = self._refs.get(win, [])
            if isinstance(ref, int) and 0 <= ref < len(refs):
                widget, mine, user = refs[ref]
                e = Event(ekind, value, widget)
                if mine is not None:
                    mine(e)
                if user is not None:
                    user(e)
        elif kind == "hotkey":
            fn = self._hotkeys.get(ev[1])
            if fn is not None and ev[2]:
                fn()
        elif kind == "suspend":
            data = self._on_suspend() if self._on_suspend else None
            _ui.save_state(self._h, b"" if data is None else _ui.json_dumps(data).encode())
        elif kind == "frame":
            if self._on_frame:
                self._on_frame(ev[2])
        elif kind == "close":
            if self._on_close:
                self._on_close(ev[1])
            else:
                self.close_window(ev[1])
                if not self._windows:
                    _ui.quit(self._h)
        elif kind == "media":
            if self._on_media:
                self._on_media(ev[1], ev[2])
        elif kind == "background":
            if self._on_background:
                self._on_background(ev[1], ev[2])
        elif kind == "message":
            if self._on_message:
                self._on_message(ev[1], ev[2])

    def step(self, timeout_ms=-1):
        """One loop turn: draw changes, wait for input, run handlers.
        False once the connection is closed or the app quit."""
        if not self._sync():
            return False
        events = _ui.step(self._h, timeout_ms)
        if events is None:
            return False
        for ev in events:
            self._dispatch(ev)
        return not _ui.is_closed(self._h)

    def run(self):
        """Run until the window is closed."""
        while self.step(-1):
            pass
        _ui.flush(self._h)


def _read_text(path):
    """A file's text: through open(), or the computer's storage (paths from its root)."""
    try:
        with open(path) as f:
            return f.read()
    except Exception:
        try:
            import terminal
            text = terminal.read_file(path)
        except Exception:
            text = None
        if text is None:
            raise
        return text


def load(path, handlers=None, app=None, app_id=None):
    """Open the window described by a .ui.json file. handlers maps the
    file's handler names to functions. Returns (app, window id)."""
    doc = _ui.json_loads(_read_text(path))
    if app is None:
        app = App(app_id or "dev.ecm.layout")
    layout = _Layout(doc["root"], handlers or {}, app)
    win = app.window(doc.get("window", ""), layout)
    layout._win = win
    return app, win
