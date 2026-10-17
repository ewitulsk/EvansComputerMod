--[[ Graphical apps: windows, widgets and layout (docs/gui/UI_PROTOCOL.md §13).
The Lua counterpart of Python's `ui` module (same widgets and App methods).

    local app = ui.App("dev.evan.notes")
    local notes = app:restore().notes or ui.array()
    local entry = ui.TextField{placeholder = "New note"}
    local rows = ui.List{}

    local function show()
      local items = {}
      for i, n in ipairs(notes) do items[i] = ui.Label(n) end
      rows.items = items
    end

    local function add()
      if entry.text ~= "" then
        notes[#notes + 1] = entry.text
        entry.text = ""
        show()
      end
    end

    show()
    app:window("Notes", ui.Column{gap = 8, padding = 12, children = {
      ui.Row{gap = 8, children = {entry, ui.Button("Add", {on_click = add})}},
      rows,
    }})
    app:hotkey(1, "Add note", add)
    app:on_suspend(function() return {notes = notes} end)
    app:run()

Widgets keep their own values: change them (entry.text = "", box.checked =
true, rows.items = {...}) and the window updates on the next loop turn.
Layout, text wrapping, focus and drawing run in the same Rust toolkit Rust
apps use, so both look and behave the same.

Widgets: Label, Button, TextField, TextArea, List, ScrollView, Checkbox,
Toggle, Slider, Tabs, Menu, Dialog, Image, Canvas, PixelGrid (an editable
indexed bitmap: card faces, icons) and ColorPicker. Layout: Row, Column, Stack,
Grid, Spacer, Responsive (one layout per size class: compact / medium / wide).
Arguments go in Python's order (ui.Button("Add", {on_click = add})), or all in
one table (ui.Button{text = "Add", on_click = add}). Common options: id,
width, height, grow, align ("start", "center", "end", "stretch"), padding,
gap, enabled, style ("primary", "danger" for buttons; "title", "caption" for
labels), font ("ui" / "mono"), size, color, wrap.
Handlers receive an event table with .kind, .value and .widget.

ui.load("app.ui.json", {add_note = add}) opens a window from a Visual
Programmer layout file.

Pixels: app:canvas(w, h, format) ("rgba8888" by default) makes a Canvas
widget with its own pixels. canvas:put(pixels, damage) shows new pixels at
the next loop turn; canvas:commit_at(pts_us) shows the ones put since the
last commit at a time on ui.now_us()'s clock instead (smooth video: queue
frames a little ahead), canvas:queue_flush() drops the queued ones (seek,
pause) and canvas.content = "motion" lets them travel lossy ("auto",
"exact").

JSON: a table with keys 1..n is a list; an empty table is an object unless
made with ui.array() (lists read from JSON stay lists).
]]

local native = require("_ui")

local M = {}

--- A table marked as a JSON list (so an empty one stays `[]`).
M.array = native.array
M.json_dumps = native.json_dumps
M.json_loads = native.json_loads

local COMMON = {}
for _, k in ipairs({ "width", "height", "grow", "align", "padding", "gap", "enabled", "style",
  "font", "size", "color", "wrap" }) do
  COMMON[k] = true
end

local function list_copy(t, f)
  local out = M.array()
  for i, x in ipairs(t or {}) do out[i] = f and f(x) or x end
  return out
end

-- An options table: string keys (a list of children has only 1..n).
local function is_options(t)
  if type(t) ~= "table" or getmetatable(t) ~= nil then return false end
  for k in pairs(t) do
    if type(k) ~= "number" then return true end
  end
  return false
end

-- ---------------------------------------------------------------- widgets

local Widget = {}
M.Widget = Widget

local function is_widget(x)
  return type(x) == "table" and rawget(x, "_cls") ~= nil
end

local function event(kind, value, widget)
  return setmetatable({ kind = kind, value = value, widget = widget }, {
    __tostring = function() return "Event(" .. tostring(kind) .. ", " .. tostring(value) .. ")" end,
  })
end

-- A widget class: spec = {kind, positional = {...}, props = {...},
-- events = {...}, init = fn(self, kw), fields = fn(self), internal = fn(self,
-- kind), kids = fn(self), get = {name = fn}, set = {name = fn}}.
local function widget_class(name, spec)
  local cls = spec
  cls.name = name
  cls.props = cls.props or {}
  cls.events = cls.events or {}
  cls.get = cls.get or {}
  cls.set = cls.set or {}
  local propset = {}
  for _, p in ipairs(cls.props) do propset[p] = true end
  cls.propset = propset
  cls.mt = {
    __index = function(t, k)
      local g = cls.get[k]
      if g then return g(t) end
      if COMMON[k] then return rawget(t, "_opts")[k] end
      return Widget[k]
    end,
    __newindex = function(t, k, v)
      local s = cls.set[k]
      if s then
        s(t, v)
      elseif COMMON[k] then
        rawget(t, "_opts")[k] = v
      else
        rawset(t, k, v)
      end
    end,
    __tostring = function(t) return name .. "(" .. tostring(rawget(t, "id") or "") .. ")" end,
  }
  M[name] = function(...)
    local args = { ... }
    local n = select("#", ...)
    local kw = {}
    if n > 0 and is_options(args[n]) then
      for k, v in pairs(args[n]) do kw[k] = v end
      n = n - 1
    end
    for i, p in ipairs(cls.positional or {}) do
      if i <= n and args[i] ~= nil and kw[p] == nil then kw[p] = args[i] end
    end
    local self = setmetatable({}, cls.mt)
    rawset(self, "_cls", cls)
    rawset(self, "_opts", {})
    rawset(self, "_handlers", {})
    rawset(self, "id", kw.id)
    kw.id = nil
    if cls.init then cls.init(self, kw) end
    for k, v in pairs(kw) do
      local ev = type(k) == "string" and k:match("^on_(.*)$")
      local known_event = false
      if ev then
        for _, e in ipairs(cls.events) do
          if e == ev then known_event = true end
        end
      end
      if known_event then
        self._handlers[ev] = v
      elseif COMMON[k] or propset[k] then
        self[k] = v
      else
        error(name .. " got an unexpected option " .. tostring(k), 2)
      end
    end
    return self
  end
  return cls
end

--- Set the handler for "click", "change", "submit" or "select".
function Widget:on(kind, fn)
  self._handlers[kind] = fn
  return self
end

function Widget:_tree(refs)
  local cls = self._cls
  local body = {}
  if rawget(self, "id") ~= nil then body.id = rawget(self, "id") end
  for k, v in pairs(self._opts) do body[k] = v end
  if cls.fields then
    for k, v in pairs(cls.fields(self)) do body[k] = v end
  end
  for _, kind in ipairs(cls.events) do
    local mine = cls.internal and cls.internal(self, kind)
    local user = self._handlers[kind]
    if mine ~= nil or user ~= nil then
      refs[#refs + 1] = { self, mine, user }
      body["on_" .. kind] = #refs - 1
    end
  end
  local kids = cls.kids and cls.kids(self)
  if kids ~= nil then
    local out = M.array()
    for i, c in ipairs(kids) do out[i] = c:_tree(refs) end
    body.children = out
  end
  return { [cls.kind] = body }
end

--- Send the values again at the next sync (after window(), whose
--- serialisation only declared the surface).
function Widget:_repush()
  if rawget(self, "_push") ~= nil then rawset(self, "_push", true) end
  local kids = self._cls.kids and self._cls.kids(self)
  for _, c in ipairs(kids or {}) do c:_repush() end
end

local function text_init(self, kw)
  rawset(self, "text", tostring(kw.text == nil and "" or kw.text))
  kw.text = nil
end

local function text_fields(self)
  return { text = tostring(self.text) }
end

widget_class("Label", { kind = "label", positional = { "text" }, props = { "text" }, events = { "click" },
  init = text_init, fields = text_fields })

widget_class("Button", { kind = "button", positional = { "text" }, props = { "text" }, events = { "click" },
  init = text_init, fields = text_fields })

-- .text follows what the player types.
local function textfield_init(self, kw)
  rawset(self, "placeholder", kw.placeholder or "")
  rawset(self, "password", kw.password and true or false)
  local text = tostring(kw.text == nil and "" or kw.text)
  rawset(self, "_text", text)
  rawset(self, "_push", text ~= "")
  kw.text, kw.placeholder, kw.password = nil, nil, nil
end

local function textfield_fields(self)
  local f = {}
  if self.placeholder ~= "" and self.placeholder ~= nil then f.placeholder = self.placeholder end
  if self.password then f.password = true end
  if rawget(self, "_push") then
    f.text = rawget(self, "_text")
    rawset(self, "_push", false)
  end
  return f
end

local text_get = { text = function(self) return rawget(self, "_text") end }
local text_set = {
  text = function(self, v)
    rawset(self, "_text", tostring(v))
    rawset(self, "_push", true)
  end,
}

widget_class("TextField", {
  kind = "textfield", positional = { "text", "placeholder", "password" },
  props = { "placeholder", "password" }, events = { "change", "submit" },
  init = textfield_init, fields = textfield_fields, get = text_get, set = text_set,
  internal = function(self, kind)
    if kind == "change" then
      return function(ev) rawset(self, "_text", ev.value) end
    end
  end,
})

-- A multi-line text editor (selection, Ctrl+C/X/V, scrolling). .text follows
-- the edits; line_numbers = true shows a gutter.
widget_class("TextArea", {
  kind = "textarea", positional = { "text", "placeholder", "line_numbers" },
  props = { "placeholder", "line_numbers" }, events = { "change" },
  init = function(self, kw)
    rawset(self, "line_numbers", kw.line_numbers and true or false)
    kw.line_numbers = nil
    textfield_init(self, kw)
  end,
  fields = function(self)
    local f = textfield_fields(self)
    if self.line_numbers then f.line_numbers = true end
    return f
  end,
  get = text_get, set = text_set,
  internal = function(self, kind)
    return function(ev) rawset(self, "_text", ev.value) end
  end,
})

local function toggle_spec(kind)
  return {
    kind = kind, positional = { "text", "checked" }, props = { "text" }, events = { "change" },
    init = function(self, kw)
      rawset(self, "text", tostring(kw.text == nil and "" or kw.text))
      rawset(self, "_checked", kw.checked and true or false)
      rawset(self, "_push", true)
      kw.text, kw.checked = nil, nil
    end,
    fields = function(self)
      local f = { text = self.text }
      if rawget(self, "_push") then
        f.checked = rawget(self, "_checked")
        rawset(self, "_push", false)
      end
      return f
    end,
    get = { checked = function(self) return rawget(self, "_checked") end },
    set = {
      checked = function(self, v)
        rawset(self, "_checked", v and true or false)
        rawset(self, "_push", true)
      end,
    },
    internal = function(self)
      return function(ev) rawset(self, "_checked", ev.value and true or false) end
    end,
  }
end

widget_class("Checkbox", toggle_spec("checkbox"))
widget_class("Toggle", toggle_spec("toggle"))

-- Whole-number slider from min to max in steps of step.
widget_class("Slider", {
  kind = "slider", positional = { "min", "max", "value", "step" }, events = { "change" },
  init = function(self, kw)
    local lo = math.floor(kw.min or 0)
    rawset(self, "min", lo)
    rawset(self, "max", math.floor(kw.max or 100))
    rawset(self, "step", math.floor(kw.step or 1))
    rawset(self, "_value", math.floor(kw.value == nil and lo or kw.value))
    rawset(self, "_push", true)
    kw.min, kw.max, kw.step, kw.value = nil, nil, nil, nil
  end,
  fields = function(self)
    local f = { min = self.min, max = self.max, step = self.step }
    if rawget(self, "_push") then
      f.value = rawget(self, "_value")
      rawset(self, "_push", false)
    end
    return f
  end,
  get = { value = function(self) return rawget(self, "_value") end },
  set = {
    value = function(self, v)
      rawset(self, "_value", math.floor(v))
      rawset(self, "_push", true)
    end,
  },
  internal = function(self)
    return function(ev) rawset(self, "_value", ev.value) end
  end,
})

-- Lists, menus and tabs: .selected is the chosen row (0-based, as the toolkit
-- and Python count; nil = none).
local function selectable(spec)
  spec.events = { "select" }
  spec.get = { selected = function(self) return rawget(self, "_selected") end }
  spec.set = {
    selected = function(self, i)
      rawset(self, "_selected", i)
      rawset(self, "_push", true)
    end,
  }
  local init = spec.init
  spec.init = function(self, kw)
    rawset(self, "_selected", kw.selected)
    rawset(self, "_push", kw.selected ~= nil)
    kw.selected = nil
    init(self, kw)
  end
  local fields = spec.fields
  spec.fields = function(self)
    local f = fields and fields(self) or {}
    if rawget(self, "_push") then
      f.selected = rawget(self, "_selected")
      rawset(self, "_push", false)
    end
    return f
  end
  spec.internal = function(self)
    return function(ev) rawset(self, "_selected", ev.value) end
  end
  return spec
end

-- Rows of widgets (or strings).
widget_class("List", selectable({
  kind = "list", positional = { "items", "selected" }, props = { "items" },
  init = function(self, kw)
    rawset(self, "items", kw.items or {})
    kw.items = nil
  end,
  kids = function(self)
    local out = {}
    for i, x in ipairs(self.items) do
      out[i] = is_widget(x) and x or M.Label(tostring(x))
    end
    return out
  end,
}))

widget_class("Menu", selectable({
  kind = "menu", positional = { "items", "selected" }, props = { "items" },
  init = function(self, kw)
    rawset(self, "items", list_copy(kw.items, tostring))
    kw.items = nil
  end,
  fields = function(self) return { items = list_copy(self.items, tostring) } end,
}))

-- ui.Tabs({"One", "Two"}, {page_one, page_two}).
widget_class("Tabs", selectable({
  kind = "tabs", positional = { "titles", "pages", "selected" }, props = { "titles", "pages" },
  init = function(self, kw)
    rawset(self, "titles", list_copy(kw.titles, tostring))
    rawset(self, "pages", kw.pages or {})
    kw.titles, kw.pages = nil, nil
  end,
  fields = function(self) return { items = list_copy(self.titles, tostring) } end,
  kids = function(self) return self.pages end,
}))

local function container(kind, extra)
  local spec = {
    kind = kind, positional = { "children" }, props = { "children" },
    init = function(self, kw)
      rawset(self, "children", kw.children or {})
      kw.children = nil
    end,
    kids = function(self) return self.children end,
  }
  for k, v in pairs(extra or {}) do spec[k] = v end
  return spec
end

widget_class("Row", container("row"))
widget_class("Column", container("column"))
widget_class("Stack", container("stack"))

-- columns: a number, or {compact, medium, wide}.
widget_class("Grid", container("grid", {
  positional = { "children", "columns" }, props = { "children", "columns" },
  init = function(self, kw)
    rawset(self, "children", kw.children or {})
    rawset(self, "columns", kw.columns or 2)
    kw.children, kw.columns = nil, nil
  end,
  fields = function(self)
    local c = self.columns
    if type(c) == "table" then return { columns = list_copy(c) } end
    return { columns = math.floor(c) }
  end,
}))

widget_class("Spacer", { kind = "spacer" })

widget_class("ScrollView", {
  kind = "scroll", positional = { "child" }, props = { "child" },
  init = function(self, kw)
    rawset(self, "child", kw.child)
    kw.child = nil
  end,
  kids = function(self) return self.child and { self.child } or {} end,
})

-- A modal card over its area: put it last in a Stack.
widget_class("Dialog", container("dialog", {
  positional = { "title", "children" }, props = { "children", "title" },
  init = function(self, kw)
    rawset(self, "title", tostring(kw.title or ""))
    rawset(self, "children", kw.children or {})
    kw.title, kw.children = nil, nil
  end,
  fields = function(self) return { text = self.title } end,
}))

-- One layout per size class (compact < 400 lu wide, medium < 800, wide).
widget_class("Responsive", {
  kind = "responsive", positional = { "compact", "medium", "wide" }, props = { "compact", "medium", "wide" },
  init = function(self, kw)
    rawset(self, "compact", kw.compact)
    rawset(self, "medium", kw.medium)
    rawset(self, "wide", kw.wide)
    kw.compact, kw.medium, kw.wide = nil, nil, nil
  end,
  kids = function(self)
    local first = self.compact or self.medium or self.wide
    local c = self.compact or first
    local m = self.medium or c
    return { c, m, self.wide or m }
  end,
})

local function sized_init(names)
  return function(self, kw)
    for _, k in ipairs(names) do
      rawset(self, k, math.floor(kw[k] or (k == "scale" and 1 or 0)))
      kw[k] = nil
    end
    self.width = kw.width or 0
    self.height = kw.height or 0
    kw.width, kw.height = nil, nil
  end
end

widget_class("Image", {
  kind = "image", positional = { "resource", "width", "height" }, props = { "resource" }, events = { "click" },
  init = sized_init({ "resource" }),
  fields = function(self) return { resource = self.resource } end,
})

local CONTENT = { auto = 0, exact = 1, motion = 2 }

local function canvas_owner(self)
  local app = rawget(self, "_canvas_app")
  if app == nil then error("this Canvas has no pixels of its own (make it with app:canvas(w, h))", 3) end
  return app
end

--- Shows a canvas resource. Made with app:canvas(w, h), it has its own
--- pixels: put(), commit_at(), queue_flush() and the content hint.
widget_class("Canvas", {
  kind = "canvas", positional = { "resource", "width", "height", "scale" }, props = { "resource", "scale" },
  events = { "click" },
  init = sized_init({ "resource", "scale" }),
  fields = function(self) return { resource = self.resource, scale = self.scale } end,
  get = { content = function(t) return rawget(t, "_content") or "auto" end },
  set = {
    content = function(t, v)
      if CONTENT[v] == nil then error("content is 'auto', 'exact' or 'motion'", 3) end
      rawset(t, "_content", v)
      native.set_content(canvas_owner(t)._h, t.resource, CONTENT[v])
    end,
  },
})

--- New pixels for a Canvas from app:canvas: a string of bytes in the canvas
--- format, the whole canvas; damage = the changed rects {{x, y, w, h}, ...}
--- (nil: all). They show at the next loop turn, or at commit_at's time.
function Widget:put(pixels, damage)
  local app = canvas_owner(self)
  native.put_canvas(app._h, self.resource, pixels, damage)
end

--- Show the pixels put since the last commit at pts_us on ui.now_us()'s
--- clock (not at the next loop turn). Frames queue in time order, a little
--- ahead (about half a second at most).
function Widget:commit_at(pts_us)
  native.commit_at(canvas_owner(self)._h, math.floor(pts_us))
end

--- Drop the frames queued with commit_at that aren't shown yet (seek, pause).
function Widget:queue_flush()
  native.queue_flush(canvas_owner(self)._h)
end

--- The monotonic clock in microseconds: the clock commit_at times are on.
function M.now_us() return native.now_us() end

-- One hex digit per texel (indices 0-15), the compact wire form.
local function hex_texels(values)
  local out = {}
  for i, v in ipairs(values) do out[i] = string.format("%x", math.floor(v) % 16) end
  return table.concat(out)
end

-- An editable indexed bitmap: width x height texels (not lu), each a palette
-- index (up to 16 colours, 0xRRGGBB), drawn zoom lu per texel (auto-fit
-- without zoom). The player paints with the pointer using tool ("pencil",
-- "fill", "eyedropper" or "line") in palette index .color, mirrored
-- left-right with mirror_h; texels whose mask entry is non-zero are locked
-- (drawn hatched, never painted). .pixels follows the edits (on_change gets
-- the changed texels as {x, y, index} lists); the eyedropper sets .color and
-- fires on_select with the picked index. ui.PixelGrid(width, height, pixels,
-- palette, {zoom = 6, color = 1, tool = "fill", mask = {...}, ...}).
widget_class("PixelGrid", {
  kind = "pixel_grid", positional = { "width", "height", "pixels", "palette" },
  props = { "palette", "zoom", "tool", "mirror_h", "grid", "mask", "pixels", "color" },
  events = { "change", "select" },
  init = function(self, kw)
    rawset(self, "cols", math.floor(kw.width or 16))
    rawset(self, "rows", math.floor(kw.height or 16))
    rawset(self, "palette", kw.palette and list_copy(kw.palette, math.floor) or nil)
    rawset(self, "zoom", kw.zoom and math.floor(kw.zoom) or nil)
    rawset(self, "tool", tostring(kw.tool or "pencil"))
    rawset(self, "mirror_h", kw.mirror_h and true or false)
    rawset(self, "grid", kw.grid == nil or kw.grid)
    rawset(self, "mask", kw.mask and list_copy(kw.mask, math.floor) or nil)
    rawset(self, "_pixels", {})
    self.pixels = kw.pixels or {}
    self.color = math.floor(kw.color or 0)
    kw.width, kw.height, kw.palette, kw.zoom, kw.tool, kw.mirror_h, kw.grid, kw.mask, kw.pixels, kw.color =
      nil, nil, nil, nil, nil, nil, nil, nil, nil, nil
  end,
  -- width / height are the bitmap's size in texels (the lu size is zoom *
  -- texels, or the space the layout gives the grid).
  get = {
    width = function(self) return rawget(self, "cols") end,
    height = function(self) return rawget(self, "rows") end,
    -- The bitmap as a list of palette indices, row by row (width * height).
    pixels = function(self) return rawget(self, "_pixels") end,
  },
  set = {
    width = function(self, v) rawset(self, "cols", math.floor(v)) end,
    height = function(self, v) rawset(self, "rows", math.floor(v)) end,
    pixels = function(self, values)
      local n = rawget(self, "cols") * rawget(self, "rows")
      local px = M.array()
      for i = 1, n do
        local v = values[i]
        px[i] = v and (math.floor(v) % 16) or 0
      end
      rawset(self, "_pixels", px)
      rawset(self, "_push", true)
    end,
  },
  fields = function(self)
    local f = { cols = self.cols, rows = self.rows, tool = self.tool, mirror_h = self.mirror_h, grid = self.grid }
    if self.palette ~= nil then f.palette = list_copy(self.palette) end
    if self.zoom ~= nil then f.zoom = self.zoom end
    if self.mask ~= nil then f.mask = hex_texels(self.mask) end
    if rawget(self, "_push") then
      f.pixels = hex_texels(rawget(self, "_pixels"))
      rawset(self, "_push", false)
    end
    return f
  end,
  internal = function(self, kind)
    if kind == "change" then
      return function(ev)
        local px, cols = rawget(self, "_pixels"), rawget(self, "cols")
        for _, t in ipairs(ev.value) do px[t[2] * cols + t[1] + 1] = t[3] end
      end
    end
    return function(ev) rawget(self, "_opts").color = ev.value end
  end,
})

--- The palette index at texel (x, y) (0-based, as the toolkit counts).
function Widget:pixel(x, y)
  return rawget(self, "_pixels")[y * rawget(self, "cols") + x + 1]
end

--- Paint texel (x, y) from the program (shown at the next loop turn).
function Widget:set_pixel(x, y, index)
  rawget(self, "_pixels")[y * rawget(self, "cols") + x + 1] = math.floor(index) % 16
  rawset(self, "_push", true)
end

-- An RGB colour picker: a hue strip, a saturation/value square and an
-- editable #rrggbb field. .value is the colour as 0xRRGGBB and follows every
-- drag and valid hex edit (on_change); .hex is "#rrggbb".
widget_class("ColorPicker", {
  kind = "color_picker", positional = { "value" }, props = { "value" }, events = { "change" },
  init = function(self, kw)
    rawset(self, "_value", math.floor(kw.value or 0) % 0x1000000)
    rawset(self, "_push", true)
    kw.value = nil
  end,
  get = {
    value = function(self) return rawget(self, "_value") end,
    hex = function(self) return string.format("#%06x", rawget(self, "_value")) end,
  },
  set = {
    value = function(self, v)
      rawset(self, "_value", math.floor(v) % 0x1000000)
      rawset(self, "_push", true)
    end,
  },
  fields = function(self)
    local f = {}
    if rawget(self, "_push") then
      f.value = rawget(self, "_value")
      rawset(self, "_push", false)
    end
    return f
  end,
  internal = function(self)
    return function(ev) rawset(self, "_value", ev.value) end
  end,
})

-- ---------------------------------------------------------------- layouts

local function is_list(t)
  if type(t) ~= "table" then return false end
  local mt = getmetatable(t)
  if mt and mt.__jsonarray then return true end
  return t[1] ~= nil
end

-- A window from a .ui.json file: the tree is sent as written. Change a widget
-- with layout:set("id", {text = ...}) (drawn at the next loop turn) and read
-- what the player did with layout:value("id").
local Layout = {}
Layout.__index = Layout

local function new_layout(root, handlers, app)
  return setmetatable({ root = root, handlers = handlers or {}, _app = app, _win = 0 }, Layout)
end

local CHILD_KEYS = { children = true, child = true, compact = true, medium = true, wide = true }

--- kind, props of the widget with this id, or nil.
function Layout:_find(id)
  local function walk(node)
    if is_list(node) then
      for _, x in ipairs(node) do
        local k, p = walk(x)
        if p ~= nil then return k, p end
      end
    elseif type(node) == "table" then
      local kind, props = next(node)
      if kind ~= nil and next(node, kind) == nil and type(props) == "table" then
        if props.id == id then return kind, props end
        for k, v in pairs(props) do
          if CHILD_KEYS[k] then
            local k2, p2 = walk(v)
            if p2 ~= nil then return k2, p2 end
          end
        end
      end
    end
    return nil, nil
  end
  return walk(self.root)
end

--- The props table of the widget with this id (nil if there is none).
function Layout:find(id)
  local _, p = self:_find(id)
  return p
end

function Layout:get(id, prop, default)
  local node = self:find(id)
  if node == nil or node[prop] == nil then return default end
  return node[prop]
end

--- Change props of the widget with this id, e.g. set("count", {text = "3"}).
function Layout:set(id, props)
  local node = self:find(id)
  if node == nil then error("no widget with id " .. tostring(id), 2) end
  for k, v in pairs(props) do
    if (k == "text" or k == "placeholder") and type(v) ~= "string" then v = tostring(v) end
    node[k] = v
  end
end

--- What the widget holds now: a text field's text, a checkbox's or toggle's
--- state, a slider's value, a list's / tab's selection, a pixel grid's bitmap,
--- a colour picker's 0xRRGGBB.
function Layout:value(id)
  local kind, node = self:_find(id)
  if node == nil then return nil end
  local state = self._app and native.widget_state(self._app._h, self._win, id)
  if kind == "checkbox" or kind == "toggle" then
    if state then return state.checked end
    return node.checked and true or false
  end
  if kind == "slider" then
    if state then return state.value end
    return math.floor(node.value or node.min or 0)
  end
  if kind == "list" or kind == "tabs" or kind == "menu" then
    if state then return state.selected end
    return node.selected
  end
  if kind == "textfield" then
    if state then return state.text end
    return node.text or ""
  end
  if kind == "pixel_grid" then
    if state then return state.pixels end
    return node.pixels or M.array()
  end
  if kind == "color_picker" then
    if state then return state.value end
    return math.floor(node.value or 0)
  end
  return node.text or ""
end

function Layout:_repush() end

function Layout:_tree(refs)
  local handlers = self.handlers
  local function walk(node)
    if type(node) ~= "table" then return node end
    if is_list(node) then
      local out = M.array()
      for i, x in ipairs(node) do out[i] = walk(x) end
      return out
    end
    local out = {}
    for k, v in pairs(node) do
      if type(k) == "string" and k:sub(1, 3) == "on_" and type(v) == "string" then
        refs[#refs + 1] = { nil, nil, handlers[v] }
        out[k] = #refs - 1
      else
        out[k] = walk(v)
      end
    end
    return out
  end
  return walk(self.root)
end

-- ---------------------------------------------------------------- App

local App = {}
App.__index = App
M.AppClass = App

--- A GUI app. ui.App("dev.you.app") connects to the compositor; it fails when
--- no GUI is running (console mode).
function M.App(app_id, shell)
  local app = setmetatable({}, App)
  app.app_id = app_id
  app._h = native.connect(app_id, shell and 1 or 0)
  app._windows, app._refs, app._hotkeys = {}, {}, {}
  return app
end

--- The table saved by on_suspend in the previous run ({} if none).
function App:restore()
  local data = native.restore(self._h)
  if data == nil or data == "" then return {} end
  local ok, value = pcall(native.json_loads, data)
  if ok and type(value) == "table" and not is_list(value) then return value end
  return {}
end

local function serialise(root)
  local refs = {}
  local tree = root:_tree(refs)
  return tree, refs
end

--- Open a window showing root; returns its id.
function App:window(title, root)
  local tree, refs = serialise(root)
  local win = native.window(self._h, title, tree)
  -- The window is drawn after the first step's sync, which serialises again:
  -- keep the widgets' initial values for it.
  root:_repush()
  self._windows[win] = root
  self._refs[win] = refs
  return win
end

function App:set_root(win, root) self._windows[win] = root end

--- A Canvas widget w x h lu with its own w x h pixels in format ("rgba8888"
--- (default), "rgb565" or "indexed8"); fill it with canvas:put().
function App:canvas(w, h, format)
  local res = native.canvas(self._h, math.floor(w), math.floor(h), format or "rgba8888")
  local c = M.Canvas(res, math.floor(w), math.floor(h), 1)
  rawset(c, "_canvas_app", self)
  return c
end

function App:_first()
  local first
  for w in pairs(self._windows) do
    if first == nil or w < first then first = w end
  end
  return first or 0
end

--- The layout of a window opened with ui.load (nil for other windows).
function App:layout(win)
  local root = self._windows[win or self:_first()]
  if getmetatable(root) == Layout then return root end
  return nil
end

function App:close_window(win)
  self._windows[win] = nil
  self._refs[win] = nil
  native.close_window(self._h, win)
end

--- width, height, size class of a window.
function App:size(win) return native.window_size(self._h, win or self:_first()) end

--- Claim ePhone hotkey slot 1-6; fn() runs when it is pressed.
function App:hotkey(slot, label, fn)
  native.hotkey(self._h, slot, label)
  self._hotkeys[slot] = fn
end

--- fn() returns a table to keep; restore() gives it back after a restart.
function App:on_suspend(fn) self._on_suspend = fn end
--- fn(time_ms) after request_frame(), at the next refresh while visible.
function App:on_frame(fn) self._on_frame = fn end
--- fn(win) when the player closes a window (default: close it; quit after the last).
function App:on_close(fn) self._on_close = fn end
--- fn(name, json_text) for other messages (clipboard data, picked files, ...).
function App:on_message(fn) self._on_message = fn end

function App:request_frame(win) native.request_frame(self._h, win or self:_first()) end

function App:notify(title, body, tag, actions)
  native.send(self._h, "Notify", { tag = tag or "", title = title, body = body or "", actions = list_copy(actions) })
end

function App:copy(text)
  native.send(self._h, "ClipboardSet", { mime = "text/plain;charset=utf-8", data = text })
end

--- Ask for the clipboard; it arrives as on_message("ClipboardData", ...).
function App:paste()
  native.send(self._h, "ClipboardRequest", { mime = "text/plain;charset=utf-8" })
end

--- pick_file{save=, types=, suggested=, req=}
function App:pick_file(o)
  o = o or {}
  native.send(self._h, "PickFile", { req = o.req or 1, mode = o.save and "save" or "open",
    types = list_copy(o.types), suggested = o.suggested or "" })
end

function App:open(path) native.send(self._h, "Open", { path = path }) end

function App:launch(app_id, args)
  native.send(self._h, "Launch", { app_id = app_id, args = list_copy(args) })
end

function App:background_done() native.send(self._h, "BackgroundDone", {}) end

function App:quit() native.quit(self._h) end

function App:_sync()
  for win, root in pairs(self._windows) do
    -- Not configured yet means nothing is drawn: the tree given to window()
    -- stands and the widgets' pushes wait for the first draw.
    local w = native.window_size(self._h, win)
    if w ~= 0 then
      local tree, refs = serialise(root)
      native.set_root(self._h, win, tree)
      self._refs[win] = refs
    end
  end
  return native.render(self._h)
end

function App:_dispatch(ev)
  local kind = ev[1]
  if kind == "widget" then
    local win, ref, ekind, value = ev[2], ev[3], ev[4], ev[5]
    local refs = self._refs[win] or {}
    if type(ref) == "number" and ref >= 0 and ref < #refs then
      local r = refs[ref + 1]
      local e = event(ekind, value, r[1])
      if r[2] ~= nil then r[2](e) end
      if r[3] ~= nil then r[3](e) end
    end
  elseif kind == "hotkey" then
    local fn = self._hotkeys[ev[2]]
    if fn ~= nil and ev[3] then fn() end
  elseif kind == "suspend" then
    local data = self._on_suspend and self._on_suspend()
    native.save_state(self._h, data == nil and "" or native.json_dumps(data))
  elseif kind == "frame" then
    if self._on_frame then self._on_frame(ev[3]) end
  elseif kind == "close" then
    if self._on_close then
      self._on_close(ev[2])
    else
      self:close_window(ev[2])
      if next(self._windows) == nil then native.quit(self._h) end
    end
  elseif kind == "message" then
    if self._on_message then self._on_message(ev[2], ev[3]) end
  end
end

--- One loop turn: draw changes, wait for input, run handlers.
--- false once the connection is closed or the app quit.
function App:step(timeout_ms)
  if not self:_sync() then return false end
  local events = native.step(self._h, timeout_ms or -1)
  if events == nil then return false end
  for _, ev in ipairs(events) do self:_dispatch(ev) end
  return not native.is_closed(self._h)
end

--- Run until the window is closed.
function App:run()
  while self:step(-1) do end
  native.flush(self._h)
end

-- A file's text: through io.open, or the computer's storage (paths from its root).
local function read_text(path)
  local f = io.open(path, "rb")
  if f then
    local s = f:read("*a")
    f:close()
    return s
  end
  local ok, shell = pcall(require, "shell")
  local text = ok and type(shell) == "table" and shell.read_file(path) or nil
  if text == nil then error("can't read " .. path, 3) end
  return text
end

--- Open the window described by a .ui.json file. handlers maps the file's
--- handler names to functions. Returns app, window id.
function M.load(path, handlers, app, app_id)
  local doc = native.json_loads(read_text(path))
  app = app or M.App(app_id or "dev.ecm.layout")
  local layout = new_layout(doc.root, handlers, app)
  local win = app:window(doc.window or "", layout)
  layout._win = win
  return app, win
end

return M
