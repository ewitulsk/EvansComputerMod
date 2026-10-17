--[[ Peripherals: blocks next to the computer and modules in its bays.
The Lua counterpart of Python's `peripheral` module (same functions).

    peripheral.names()                 -- {"left", "left_bay_1"}
    local link = peripheral.find("redstone_link")
    link.set_channel(0, "minecraft:red_dye", "minecraft:red_dye", "tx")
    link.set_output(0, 15)             -- link:set_output(0, 15) works too

    local event, attachment, channel, power, old = peripheral.pull_event("redstone_link", 5)

Attachment names: sides "front", "back", "left", "right", "top", "bottom"
(left/right as for redstone sides) and bay slots "left_bay_1", "left_bay_2",
"right_bay_1", "right_bay_2".

Events come back as several values (event, attachment, args...). Built-in events:
    "peripheral", name, type        something was attached
    "peripheral_detach", name       it went away

A peripheral's error is raised as a Lua error (catch it with pcall).
]]

local native = require("_peripheral")

local M = {}

local function entry(name, ty)
  return { name = name, type = ty, name, ty }
end

--- { {name=, type=}, ... } for everything attached to this computer
--- (each entry is also {name, type}).
function M.attached()
  local out = {}
  for i, p in ipairs(native.list()) do
    out[i] = entry(p[1], p[2])
  end
  return out
end
M.list = M.attached

--- Names of all attached peripherals.
function M.names()
  local out = {}
  for i, p in ipairs(native.list()) do out[i] = p[1] end
  return out
end

function M.is_present(name)
  for _, p in ipairs(native.list()) do
    if p[1] == name then return true end
  end
  return false
end

--- Type of the peripheral attached as `name`, or nil.
function M.get_type(name)
  for _, p in ipairs(native.list()) do
    if p[1] == name then return p[2] end
  end
  return nil
end

--- Method names of the peripheral attached as `name`, or nil.
function M.get_methods(name)
  local ok, _, methods = native.methods(name)
  if ok then return methods end
  return nil
end

--- type, method names of the peripheral attached as `name` (an error if there is none).
function M.methods(name)
  local ok, ty, methods = native.methods(name)
  if not ok then error(ty, 2) end
  return ty, methods
end

--- Call `method` on the peripheral attached as `name`.
function M.call(name, method, ...)
  local ok, result = native.call(name, method, ...)
  if not ok then error(result, 2) end
  return result
end

--- The peripheral attached as `name` as a table with one function per method
--- (p.method(...) or p:method(...)), plus name, type, methods() and call(); nil
--- if nothing is attached as `name`.
function M.wrap(name)
  local ok, ty, methods = native.methods(name)
  if not ok then return nil end
  local p = { name = name, type = ty }
  local function strip(first, ...)
    if rawequal(first, p) then return ... end
    return first, ...
  end
  for _, m in ipairs(methods) do
    p[m] = function(...) return M.call(name, m, strip(...)) end
  end
  local list = {}
  for i, m in ipairs(methods) do list[i] = m end
  p.methods = function() local c = {} for i, m in ipairs(list) do c[i] = m end return c end
  p.call = function(...) return M.call(name, strip(...)) end
  return setmetatable(p, {
    __tostring = function() return "peripheral " .. ty .. " '" .. name .. "'" end,
  })
end

--- Every attached peripheral of `type`, in attachment order.
function M.find_all(ty)
  local out = {}
  for _, p in ipairs(native.list()) do
    if p[2] == ty then
      local w = M.wrap(p[1])
      if w then out[#out + 1] = w end
    end
  end
  return out
end

--- First attached peripheral of `type`, or nil.
function M.find(ty)
  for _, p in ipairs(native.list()) do
    if p[2] == ty then
      local w = M.wrap(p[1])
      if w then return w end
    end
  end
  return nil
end

--- Wait for the next event (named `filter`, if given) and return it as
--- event, attachment, args...; other events are discarded while waiting.
--- `timeout` in seconds: nil waits forever, 0 only checks. nil on timeout.
function M.pull_event(filter, timeout)
  local ms = -1
  if timeout ~= nil then
    ms = math.min(math.max(0, math.floor(timeout * 1000)), 2147483647)
  end
  return native.wait_event(filter, ms)
end
M.wait_event = M.pull_event

return M
