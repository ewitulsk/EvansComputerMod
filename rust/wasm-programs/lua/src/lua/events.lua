--[[ The program's event queue: keys and typed characters, timers, in-game
alarms, peripheral events and events the program queues itself, oldest
first. The Lua counterpart of Python's `events` module (same functions);
os.pullEvent and peripheral.pull_event read the same queue.

    local t = events.start_timer(2)
    while true do
      local ev, a, b = events.pull()
      if ev == "key" then print("key", keys.get_name(a)) -- a = a key code (keys.enter, keys.a, ...)
      elseif ev == "char" then print("typed", a)
      elseif ev == "timer" and a == t then break end
    end

Events: "key", code, held / "key_up", code / "char", text / "timer", id /
"alarm", id / peripheral events (see peripheral) / your own queue(name, ...).
pull(filter) discards events with other names while it waits ("terminate"
always comes through). timeout in seconds: nil waits forever, 0 only looks.
]]

local c = require("_console")

local M = {}

--- The next event as name, args... (nil on timeout).
function M.pull(filter, timeout)
  return c.pull(filter, timeout)
end

--- Add an event to the queue.
function M.queue(name, ...)
  if type(name) ~= "string" then error("bad argument #1 (string expected, got " .. type(name) .. ")", 2) end
  c.queue(name, ...)
end

--- A "timer" event after `seconds` (in whole game ticks of 0.05 s); returns its id.
function M.start_timer(seconds) return c.start_timer(seconds) end
function M.cancel_timer(id) c.cancel_timer(id) end

--- An "alarm" event when the world's clock next reaches `hour` (0 <= hour < 24); returns its id.
function M.set_alarm(hour) return c.set_alarm(hour) end
function M.cancel_alarm(id) c.cancel_alarm(id) end

--- Wait `seconds` (at least one tick); events that arrive meanwhile are discarded.
function M.sleep(seconds) c.sleep(seconds) end

return M
