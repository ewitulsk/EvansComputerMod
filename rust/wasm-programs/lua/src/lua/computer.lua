--[[ This computer: its number, its label and its clocks. The Lua
counterpart of Python's `computer` module (same functions); CC's os.* names
(os.getComputerID, os.time, os.day, os.epoch, os.clock) are built on it.

    computer.id()              -- 0, 1, 2 ...: this computer's number in the world
    computer.set_label("base") -- kept in /etc/label
    computer.time()            -- in-game hour, 0 <= t < 24 (6.0 at a world's first morning)
    computer.time("utc")       -- the real hour, UTC ("local" is UTC too)
    computer.day()             -- in-game day, 1 on the first day ("utc": days since 1970)
    computer.epoch("utc")      -- real milliseconds since 1970 (in-game: 3600 per tick)
    computer.uptime()          -- seconds since the program started
]]

local c = require("_console")

local M = {}

function M.id() return c.id() end
function M.label() return c.label() end
function M.set_label(label)
  if label ~= nil and type(label) ~= "string" then
    error("bad argument #1 (string expected, got " .. type(label) .. ")", 2)
  end
  c.set_label(label)
end
function M.uptime() return c.uptime() end
function M.time(locale) return c.time(locale) end
function M.day(locale) return c.day(locale) end
function M.epoch(locale) return c.epoch(locale) end

return M
