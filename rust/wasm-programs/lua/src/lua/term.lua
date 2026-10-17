--[[ The terminal: a character grid with a cursor and 16 colours (the
`colors` values). The Lua counterpart of Python's `term` module (same
functions), with ComputerCraft's names too, so CC programs run unchanged.

    term.clear()
    term.set_cursor_pos(1, 1)            -- term.setCursorPos(1, 1)
    term.set_text_color(colors.yellow)   -- term.setTextColour(colours.yellow)
    term.write("Hello")                  -- no wrapping; write()/print() wrap
    local w, h = term.get_size()
    term.blit("abc", "0e4", "fff")       -- text, text colours, backgrounds (blit digits)

Positions start at 1. Text written past the edge is cut off. term.redirect(t)
sends every term call (and print/write/read) to `t`, any table with the same
functions (CC names); it returns the previous target. term.native() is the
console itself, term.current() the target now.
]]

local native_c = require("_console")

-- CC name -> snake_case name of the same function.
local NAMES = {
  write = "write", blit = "blit", clear = "clear", clearLine = "clear_line",
  scroll = "scroll", getCursorPos = "get_cursor_pos", setCursorPos = "set_cursor_pos",
  getCursorBlink = "get_cursor_blink", setCursorBlink = "set_cursor_blink",
  getSize = "get_size", isColour = "is_color", isColor = "is_color",
  getTextColour = "get_text_color", getTextColor = "get_text_color",
  setTextColour = "set_text_color", setTextColor = "set_text_color",
  getBackgroundColour = "get_background_color", getBackgroundColor = "get_background_color",
  setBackgroundColour = "set_background_color", setBackgroundColor = "set_background_color",
  getPaletteColour = "get_palette_color", getPaletteColor = "get_palette_color",
  setPaletteColour = "set_palette_color", setPaletteColor = "set_palette_color",
}

local function text(s)
  if type(s) == "string" then return s end
  if type(s) == "number" then return tostring(s) end
  error("bad argument #1 (string expected, got " .. type(s) .. ")", 3)
end

local function unpack_rgb(rgb)
  return math.floor(rgb / 65536) % 256 / 255, math.floor(rgb / 256) % 256 / 255, rgb % 256 / 255
end

local function pack_rgb(r, g, b)
  local function ch(v) return math.floor(math.max(0, math.min(1, v)) * 255) end
  return ch(r) * 65536 + ch(g) * 256 + ch(b)
end

-- The console itself, as a redirect target (CC names).
local native = {
  write = function(s) native_c.write(text(s)) end,
  blit = function(t, f, b) native_c.blit(text(t), text(f), text(b)) end,
  clear = native_c.clear,
  clearLine = native_c.clear_line,
  scroll = function(n) native_c.scroll(n) end,
  getCursorPos = native_c.get_cursor_pos,
  setCursorPos = function(x, y) native_c.set_cursor_pos(x, y) end,
  getCursorBlink = native_c.get_cursor_blink,
  setCursorBlink = function(on) native_c.set_cursor_blink(on and true or false) end,
  getSize = native_c.get_size,
  isColour = function() return true end,
  getTextColour = native_c.get_text_color,
  setTextColour = function(c) native_c.set_text_color(c) end,
  getBackgroundColour = native_c.get_background_color,
  setBackgroundColour = function(c) native_c.set_background_color(c) end,
  getPaletteColour = function(c) return unpack_rgb(native_c.get_palette_color(c)) end,
  setPaletteColour = function(c, r, g, b)
    if g == nil then native_c.set_palette_color(c, r) else native_c.set_palette_color(c, pack_rgb(r, g, b)) end
  end,
}
native.isColor = native.isColour
native.getTextColor, native.setTextColor = native.getTextColour, native.setTextColour
native.getBackgroundColor, native.setBackgroundColor = native.getBackgroundColour, native.setBackgroundColour
native.getPaletteColor, native.setPaletteColor = native.getPaletteColour, native.setPaletteColour

local current = native

local M = {}

for cc, snake in pairs(NAMES) do
  local function call(...) return current[cc](...) end
  M[cc] = call
  M[snake] = call
end

--- Send term calls to `target` (a table with the term functions); returns the previous target.
function M.redirect(target)
  if type(target) ~= "table" then
    error("bad argument #1 (table expected, got " .. type(target) .. ")", 2)
  end
  if target == M then error("term is not a recommended redirect target, try term.current() instead", 2) end
  for cc in pairs(NAMES) do
    if target[cc] == nil and not cc:find("Color$") and not cc:find("Colour$") then
      error("Redirect object is missing method " .. cc .. ".", 2)
    end
  end
  local old = current
  current = target
  return old
end

function M.current() return current end
function M.native() return native end

--- The default red, green and blue (0-1) of a colour.
local DEFAULT = {
  0xF0F0F0, 0xF2B233, 0xE57FD8, 0x99B2F2, 0xDEDE6C, 0x7FCC19, 0xF2B2CC, 0x4C4C4C,
  0x999999, 0x4C99B2, 0xB266E5, 0x3366CC, 0x7F664C, 0x57A64E, 0xCC4C4C, 0x111111,
}
function M.native_palette_color(c)
  if type(c) ~= "number" or c < 1 or c >= 65536 then error("Colour out of range", 2) end
  local i = math.floor(math.log(c) / math.log(2) + 1e-9)
  return unpack_rgb(DEFAULT[i + 1])
end
M.nativePaletteColour = M.native_palette_color
M.nativePaletteColor = M.native_palette_color

--- Write `s` wrapped at word boundaries ("\n" starts a line), scrolling at the
--- bottom; returns the number of lines started (the `write` global).
function M.write_wrapped(s)
  s = text(s)
  if current == native then return native_c.write_wrapped(s) end
  -- Another target: wrap with its own functions.
  local t = current
  local w, h = t.getSize()
  local x, y = t.getCursorPos()
  local lines = 0
  local function newline()
    if y + 1 <= h then
      t.setCursorPos(1, y + 1)
    else
      t.setCursorPos(1, h)
      t.scroll(1)
    end
    x, y = t.getCursorPos()
    lines = lines + 1
  end
  while #s > 0 do
    local ws = s:match("^[ \t]+")
    if ws then
      t.write((ws:gsub("\t", " ")))
      x, y = t.getCursorPos()
      s = s:sub(#ws + 1)
    elseif s:sub(1, 1) == "\n" then
      newline()
      s = s:sub(2)
    else
      local word = s:match("^[^ \t\n]+")
      s = s:sub(#word + 1)
      if #word > w - x + 1 and x > 1 then newline() end
      while #word > w - x + 1 do
        local room = math.max(1, w - x + 1)
        t.write(word:sub(1, room))
        newline()
        word = word:sub(room + 1)
      end
      t.write(word)
      x, y = t.getCursorPos()
    end
  end
  return lines
end

return M
