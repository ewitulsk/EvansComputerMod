--[[ Values as text and back, and text on the screen. The Lua counterpart of
Python's `textutils` module (same functions), with ComputerCraft's names.

    local s = textutils.serialize({1, 2, name = "x"})   -- CC's layout; {compact = true} for one line
    textutils.unserialize(s)                             -- the table back (literals only)
    textutils.serialize_json({a = {1, 2}})               -- '{"a":[1,2]}' (serializeJSON)
    textutils.unserialize_json('{"a":[1,2]}')
    textutils.format_time(os.time(), true)              -- "13:30" ("1:30 PM" without true)
    textutils.url_encode("a b&c")                       -- "a+b%26c"
    textutils.tabulate({"Name", "Size"}, {"a", "1"})    -- columns; a colour between rows sets it
    textutils.slow_print("text", 20)                    -- 20 characters a second

Map keys come out sorted, so the text is the same every run.
]]

local c = require("_console")

local M = {}

--- A table no other table equals: serialize_json writes it as [] (an empty table is {}).
M.empty_json_array = setmetatable({}, { __tostring = function() return "[]" end })

function M.serialize(value, opts)
  local compact = type(opts) == "table" and opts.compact or false
  return c.serialize(value, compact and true or false)
end

function M.unserialize(text)
  if type(text) ~= "string" then error("bad argument #1 (string expected, got " .. type(text) .. ")", 2) end
  return c.unserialize(text)
end

function M.serialize_json(value, opts)
  local unicode = opts == true or (type(opts) == "table" and opts.unicode_strings) or false
  return c.serialize_json(value, unicode and true or false, M.empty_json_array)
end

function M.unserialize_json(text, opts)
  local v, e = c.unserialize_json(text)
  if v == nil and e then return nil, e end
  return v
end

function M.format_time(t, twenty_four)
  return c.format_time(t, twenty_four and true or false)
end

function M.url_encode(s)
  return c.url_encode(s)
end

local function term() return require("term") end

--- Write `text` a character at a time (`rate` characters a second, 20 by default).
function M.slow_write(text, rate)
  text = tostring(text)
  local delay = 1 / (rate or 20)
  for i = 1, #text do
    sleep(delay)
    write(text:sub(i, i))
  end
end

function M.slow_print(text, rate)
  M.slow_write(text, rate)
  print()
end

--- Print `text`; when the screen fills up, wait for a key before going on.
--- Returns the number of lines printed.
function M.paged_print(text, free_lines)
  local t = term()
  local _, h = t.getSize()
  local shown = free_lines or 0
  local total = 0
  for line in (tostring(text) .. "\n"):gmatch("(.-)\n") do
    if shown >= h - 1 then
      write("Press any key to continue")
      os.pullEvent("key")
      t.clearLine()
      local _, y = t.getCursorPos()
      t.setCursorPos(1, y)
      shown = 0
    end
    local n = print(line)
    shown = shown + n
    total = total + n
  end
  return total
end

local function tabulate(paged, ...)
  local t = term()
  local w = t.getSize()
  local rows, widest = {}, 0
  for i = 1, select("#", ...) do
    local row = select(i, ...)
    if type(row) == "table" then
      for _, cell in ipairs(row) do widest = math.max(widest, #tostring(cell) + 1) end
    end
    rows[#rows + 1] = row
  end
  local cols = math.max(1, math.floor(w / math.max(widest, 1)))
  local width = math.floor(w / cols)
  local out = paged and M.paged_print or print
  local lines = 0
  for _, row in ipairs(rows) do
    if type(row) == "number" then
      t.setTextColour(row)
    else
      local line = ""
      for i, cell in ipairs(row) do
        local s = tostring(cell)
        line = line .. s
        if i % cols == 0 or i == #row then
          lines = lines + out(line)
          line = ""
        else
          line = line .. (" "):rep(width - #s)
        end
      end
    end
  end
  return lines
end

--- Rows (tables of cells) in columns as wide as the widest cell; a colour
--- between rows sets the text colour from there on.
function M.tabulate(...) return tabulate(false, ...) end
function M.paged_tabulate(...) return tabulate(true, ...) end

M.serialise, M.unserialise = M.serialize, M.unserialize
M.serializeJSON, M.serialiseJSON = M.serialize_json, M.serialize_json
M.unserializeJSON, M.unserialiseJSON = M.unserialize_json, M.unserialize_json
M.formatTime, M.urlEncode = M.format_time, M.url_encode
M.slowWrite, M.slowPrint, M.pagedPrint = M.slow_write, M.slow_print, M.paged_print
M.pagedTabulate = M.paged_tabulate

return M
