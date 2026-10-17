--[[ The 16 colours, as numbers with one bit each (white = 1 ... black =
32768), so a set of colours is their sum. The Lua counterpart of Python's
`colors` module (same names); ComputerCraft's camelCase names are here too,
and `colours` is the same with British spellings (grey, lightGrey).

    term.set_text_color(colors.red)
    colors.combine(colors.red, colors.blue)   -- a set
    colors.test(set, colors.red)              -- true
    colors.to_blit(colors.red)                -- "e"   (colors.toBlit)
    colors.pack_rgb(1, 0.5, 0)                -- 0xFF7F00 (colors.packRGB)
]]

local M = {
  white = 0x1, orange = 0x2, magenta = 0x4, lightBlue = 0x8,
  yellow = 0x10, lime = 0x20, pink = 0x40, gray = 0x80,
  lightGray = 0x100, cyan = 0x200, purple = 0x400, blue = 0x800,
  brown = 0x1000, green = 0x2000, red = 0x4000, black = 0x8000,
}
M.light_blue, M.light_gray = M.lightBlue, M.lightGray

local function bits(v) return math.floor(v) % 65536 end

local function has(set, bit) return math.floor(set / bit) % 2 == 1 end

--- The set of all the colours (and sets) given.
function M.combine(...)
  local r = 0
  for i = 1, select("#", ...) do
    local v = bits(select(i, ...))
    for b = 0, 15 do
      local bit = 2 ^ b
      if has(v, bit) and not has(r, bit) then r = r + bit end
    end
  end
  return r
end

--- `set` without the colours (and sets) given.
function M.subtract(set, ...)
  local r = bits(set)
  local drop = M.combine(...)
  for b = 0, 15 do
    local bit = 2 ^ b
    if has(drop, bit) and has(r, bit) then r = r - bit end
  end
  return r
end

--- Whether `set` holds all of `colour`.
function M.test(set, colour)
  return M.subtract(colour, set) == 0
end

--- 0xRRGGBB from red, green and blue in 0..1.
function M.pack_rgb(r, g, b)
  local function ch(v) return math.floor(math.max(0, math.min(1, v)) * 255) end
  return ch(r) * 65536 + ch(g) * 256 + ch(b)
end

--- red, green, blue in 0..1 from 0xRRGGBB.
function M.unpack_rgb(rgb)
  return math.floor(rgb / 65536) % 256 / 255, math.floor(rgb / 256) % 256 / 255, rgb % 256 / 255
end

--- The blit digit ("0"-"f") of a colour.
function M.to_blit(colour)
  if type(colour) ~= "number" or colour < 1 or colour >= 65536 then
    error("Colour out of range", 2)
  end
  local i = 0
  while 2 ^ (i + 1) <= colour do i = i + 1 end
  return ("0123456789abcdef"):sub(i + 1, i + 1)
end

--- The colour of a blit digit, or nil.
function M.from_blit(digit)
  local n = type(digit) == "string" and #digit == 1 and tonumber(digit, 16)
  if not n then return nil end
  return 2 ^ n
end

M.packRGB, M.unpackRGB, M.toBlit, M.fromBlit = M.pack_rgb, M.unpack_rgb, M.to_blit, M.from_blit
--- Deprecated in CC: packRGB with three values, unpackRGB with one.
function M.rgb8(r, g, b)
  if g == nil then return M.unpack_rgb(r) end
  return M.pack_rgb(r, g, b)
end

return M
