--[[ Wireless Xbox Controller (peripheral type "xbox_controller").
The Lua counterpart of Python's `controller` module (same functions).

A connected controller is attached as controller_1 .. controller_4, its player
number. Programs see Xbox buttons and axes only: which keys drive them is set
on the controller item (sneak + right-click it).

    local pad = controller.find()        -- lowest player number, or nil
    local s = pad:state()                -- {a=false, ..., lx=0, ..., player=1}
    if pad:is_down("a") then print("jump") end
    local x, y = pad:axis("lx"), pad:axis("ly")   -- -1..1, right and up positive

    while true do
      print(controller.pull_event())     -- button controller_1 a true
    end                                  -- axis controller_1 lx -1

Buttons: a b x y lb rb back start guide ls rs dpad_up dpad_down dpad_left dpad_right
Axes: lx ly rx ry (-1..1), lt rt (0..1)
]]

local peripheral = require("peripheral")
local ecm = require("_ecm")

local M = {}
M.TYPE = "xbox_controller"
M.BUTTONS = { "a", "b", "x", "y", "lb", "rb", "back", "start", "guide", "ls", "rs",
  "dpad_up", "dpad_down", "dpad_left", "dpad_right" }
M.AXES = { "lx", "ly", "rx", "ry", "lt", "rt" }

local Controller = {}
Controller.__index = Controller
Controller.__tostring = function(c) return "Controller('" .. c.name .. "')" end

local function new(name)
  local c = setmetatable({ name = name }, Controller)
  -- Player number, 1-4.
  c.player = tonumber(name:match("_(%d+)$"))
  return c
end

function Controller:connected()
  return peripheral.get_type(self.name) == M.TYPE
end

--- Every button (boolean) and axis (number) at once, plus player.
function Controller:state() return peripheral.call(self.name, "get_state") end

function Controller:is_down(button) return peripheral.call(self.name, "is_down", button) end

function Controller:axis(name) return peripheral.call(self.name, "get_axis", name) end

--- Names of the buttons held now.
function Controller:buttons() return peripheral.call(self.name, "get_buttons") end

--- {button bit mask (bit i = BUTTONS[i + 1]), lx, ly, rx, ry (-127..127), lt, rt (0..255)}
function Controller:raw() return peripheral.call(self.name, "get_raw") end

--- Every connected controller, by player number.
function M.all()
  local pads = {}
  for _, p in ipairs(peripheral.attached()) do
    if p.type == M.TYPE then pads[#pads + 1] = new(p.name) end
  end
  table.sort(pads, function(a, b) return (a.player or 0) < (b.player or 0) end)
  return pads
end

--- The controller of `player` (1-4), or the lowest-numbered one; nil if none is connected.
function M.find(player)
  for _, pad in ipairs(M.all()) do
    if player == nil or pad.player == player then return pad end
  end
  return nil
end

--- Next controller event as "button", name, button, pressed or "axis", name,
--- axis, value; nil after `timeout` seconds. Other peripheral events are skipped.
function M.pull_event(timeout)
  local deadline = timeout and (ecm.now() + timeout)
  while true do
    local left = deadline and math.max(0, deadline - ecm.now())
    local ev = { peripheral.pull_event(nil, left) }
    if ev[1] == nil then return nil end
    if ev[1] == "controller_button" then
      return "button", (table.unpack or unpack)(ev, 2)
    end
    if ev[1] == "controller_axis" then
      return "axis", (table.unpack or unpack)(ev, 2)
    end
  end
end

return M
