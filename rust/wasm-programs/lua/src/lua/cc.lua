--[[ ComputerCraft's globals, so CC programs run unchanged. Always on: the
modules behind them (term, colors, events, computer) only load when a
program first uses them.

Where CC and standard Lua differ, CC wins:
  os.time()     in-game hour (os.time("utc") the real one; a table still gives Unix seconds)
  os.clock()    seconds since the program started, not CPU time
  print(...)    through the term (word-wrapped, current colours); returns lines printed
  io.write(...) through the term too; io.stdout:write stays raw
Added: sleep, write, printError, read, os.pullEvent(Raw), os.queueEvent,
os.startTimer, os.cancelTimer, os.setAlarm, os.cancelAlarm, os.day, os.epoch,
os.getComputerID/computerID, os.getComputerLabel/computerLabel,
os.setComputerLabel, os.sleep, os.version, _HOST. Our os.exit stays.
]]

local function lazy(name)
  return function() return require(name) end
end
local term, events, computer = lazy("term"), lazy("events"), lazy("computer")

_HOST = "EvansComputerMod (Lua 5.2)"

-- ---------------------------------------------------------------- os

local std_time = os.time

function os.pullEventRaw(filter)
  return events().pull(filter)
end

function os.pullEvent(filter)
  local ev = table.pack(events().pull(filter))
  if ev[1] == "terminate" then error("Terminated", 0) end
  return table.unpack(ev, 1, ev.n)
end

function os.queueEvent(name, ...) events().queue(name, ...) end
function os.startTimer(seconds) return events().start_timer(seconds) end
function os.cancelTimer(id) events().cancel_timer(id) end
function os.setAlarm(hour) return events().set_alarm(hour) end
function os.cancelAlarm(id) events().cancel_alarm(id) end

function os.time(locale)
  if type(locale) == "table" then return std_time(locale) end
  return computer().time(locale)
end
function os.day(locale) return computer().day(locale) end
function os.epoch(locale) return computer().epoch(locale) end
function os.clock() return computer().uptime() end

function os.getComputerID() return computer().id() end
function os.getComputerLabel() return computer().label() end
function os.setComputerLabel(label) computer().set_label(label) end
os.computerID, os.computerLabel = os.getComputerID, os.getComputerLabel

function os.version() return "CraftOS 1.9" end

-- ---------------------------------------------------------------- globals

function sleep(seconds) events().sleep(seconds) end
os.sleep = sleep

--- Write text wrapped at word boundaries; returns the number of lines started.
function write(text)
  return term().write_wrapped(tostring(text))
end

function print(...)
  local n = select("#", ...)
  local parts = {}
  for i = 1, n do parts[i] = tostring((select(i, ...))) end
  return write(table.concat(parts, " ") .. "\n")
end

function printError(...)
  local t = term()
  local old
  if t.isColour() then
    old = t.getTextColour()
    t.setTextColour(require("colors").red)
  end
  print(...)
  if old then t.setTextColour(old) end
end

io.write = function(...)
  for i = 1, select("#", ...) do
    local v = select(i, ...)
    if type(v) ~= "string" and type(v) ~= "number" then
      error("bad argument #" .. i .. " to 'write' (string expected, got " .. type(v) .. ")", 2)
    end
    write(v)
  end
  return io.stdout
end

--- Read a line typed at the keyboard, with editing (left/right, home/end,
--- backspace/delete), history (up/down through `history`, oldest first),
--- completion (`complete(partial)` -> list of endings; Tab takes the first)
--- and a starting text. `replace` shows every character as that character.
function read(replace, history, complete, default)
  local t = term()
  if replace ~= nil then replace = tostring(replace):sub(1, 1) end
  local line = type(default) == "string" and default or ""
  local pos = #line
  local hpos
  local sx, sy = t.getCursorPos()
  local w = t.getSize()
  local scroll = 0
  local shown = 0
  local choices, choice

  local function recomplete()
    choices, choice = nil, nil
    if complete and pos == #line then
      choices = complete(line)
      if choices and #choices > 0 then choice = 1 else choices = nil end
    end
  end

  local function redraw(clear)
    local room = w - sx + 1
    if pos - scroll >= room then scroll = pos - room + 1 elseif pos < scroll then scroll = pos end
    t.setCursorPos(sx, sy)
    local text = line:sub(scroll + 1, scroll + room)
    if replace then text = replace:rep(#text) end
    if clear then text = (" "):rep(math.max(shown, #text)) end
    t.write(text)
    if not clear then
      local extra = shown - #text
      if extra > 0 then t.write((" "):rep(extra)) end
      shown = #text
      if choice then
        local old = t.getTextColour()
        t.setTextColour(require("colors").gray)
        t.write(choices[choice]:sub(1, math.max(0, room - #text)))
        t.setTextColour(old)
        shown = shown + #choices[choice]
      end
    end
    t.setCursorPos(sx + pos - scroll, sy)
  end

  local keys = require("keys")
  t.setCursorBlink(true)
  recomplete()
  redraw()
  while true do
    local ev, a = os.pullEvent()
    if ev == "char" then
      line = line:sub(1, pos) .. a .. line:sub(pos + 1)
      pos = pos + 1
      recomplete()
      redraw()
    elseif ev == "paste" then
      line = line:sub(1, pos) .. a .. line:sub(pos + 1)
      pos = pos + #a
      recomplete()
      redraw()
    elseif ev == "key" then
      if a == keys.enter or a == keys.numPadEnter then
        if choice then redraw(true) choices, choice = nil, nil redraw() end
        break
      elseif a == keys.left and pos > 0 then
        pos = pos - 1 recomplete() redraw()
      elseif a == keys.right and pos < #line then
        pos = pos + 1 recomplete() redraw()
      elseif a == keys.right and choice then
        line = line .. choices[choice] pos = #line recomplete() redraw()
      elseif a == keys.tab and choice then
        line = line .. choices[choice] pos = #line recomplete() redraw()
      elseif (a == keys.up or a == keys.down) and choice and #choices > 1 then
        choice = (choice + (a == keys.up and -2 or 0)) % #choices + 1
        redraw()
      elseif (a == keys.up or a == keys.down) and type(history) == "table" and #history > 0 then
        redraw(true)
        if a == keys.up then
          hpos = hpos and math.max(1, hpos - 1) or #history
        elseif hpos then
          hpos = hpos < #history and hpos + 1 or nil
        end
        line = hpos and history[hpos] or ""
        pos = #line
        scroll = 0
        recomplete()
        redraw()
      elseif a == keys.backspace and pos > 0 then
        redraw(true)
        line = line:sub(1, pos - 1) .. line:sub(pos + 1)
        pos = pos - 1
        recomplete()
        redraw()
      elseif a == keys.delete and pos < #line then
        redraw(true)
        line = line:sub(1, pos) .. line:sub(pos + 2)
        recomplete()
        redraw()
      elseif a == keys.home then
        pos = 0 recomplete() redraw()
      elseif a == keys["end"] then
        pos = #line recomplete() redraw()
      end
    end
  end
  t.setCursorBlink(false)
  local _, cy = t.getCursorPos()
  local _, h = t.getSize()
  if cy >= h then
    t.scroll(1)
    t.setCursorPos(1, h)
  else
    t.setCursorPos(1, cy + 1)
  end
  return line
end
