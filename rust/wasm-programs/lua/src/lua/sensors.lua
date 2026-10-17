--[[ Wired sensors: lidars wired to a Wired Sensor Module in one of the bays.
The Lua counterpart of Python's `sensors` module (same functions).

    local hub = sensors.find()              -- the first Wired Sensor Module
    print(table.concat(hub:names(), " "))   -- lidar_1 lidar_2
    local front = hub:lidar("lidar_1")
    local m = front:mount()                 -- {x=1.5, y=0, z=-0.4, yaw=0, ...}
    front:configure{az_min=-60, az_max=60, az_steps=121, el_min=-10, el_max=10, rows=3, range=24}

    local scan = front:scan()               -- take one scan and wait for it
    print(scan.seq, scan:range_at(1, 60))   -- middle row, straight ahead
    for _, p in ipairs(front:points()) do   -- hits in the computer's frame
      local x, y, z = p[1], p[2], p[3]
    end

    front:start()                           -- scan continuously
    while true do
      scan = front:wait(scan.seq)           -- the next finished scan
    end

Frames. A lidar sweeps around the structure's up axis: azimuth 0 is the
sensor's forward (out of the wall it is on, or where you looked when placing
it on a floor or ceiling), positive azimuth turns left (counter-clockwise
seen from above), positive elevation is up. Points and mounts use the
computer's frame: x = the way its screen faces, y = left, z = up, origin at
the computer's centre, in blocks.

Ranges are distances in blocks; sensors.INF (math.huge) means nothing within
range. Rows go from el_min to el_max, columns from az_min to az_max. Row and
column numbers start at 0, as in Python (scan.ranges itself is a Lua list:
scan.ranges[row * scan.columns + column + 1]).
]]

local peripheral = require("peripheral")
local ecm = require("_ecm")

local M = {}
M.INF = math.huge

--- Little-endian float32 string -> list of numbers.
function M.unpack_f32(data)
  return ecm.unpack_f32(data)
end

-- ---------------------------------------------------------------- Scan

local Scan = {}
Scan.__index = Scan
Scan.__tostring = function(s)
  return string.format("<lidar scan %d: %dx%d, %d hits>", s.seq, s.rows, s.columns, s.hits)
end

local function new_scan(info)
  local s = setmetatable({}, Scan)
  s.seq = info.seq
  s.start_tick = info.start_tick
  s.end_tick = info.end_tick
  s.rows = info.rows
  s.columns = info.columns
  s.config = info.config
  s.hits = info.hits
  s.ranges = M.unpack_f32(info.ranges)
  return s
end

function Scan:range_at(row, column)
  return self.ranges[row * self.columns + column + 1]
end

--- The ranges of one row (a list).
function Scan:row(row)
  local out, c = {}, self.columns
  for i = 1, c do out[i] = self.ranges[row * c + i] end
  return out
end

--- Azimuth of a column, degrees.
function Scan:azimuth(column)
  local cfg = self.config
  local lo, hi, n = cfg.az_min, cfg.az_max, self.columns
  if hi - lo >= 360 - 1e-9 then
    return lo + column * (hi - lo) / n
  end
  if n == 1 then return lo end
  return lo + column * (hi - lo) / (n - 1)
end

--- Elevation of a row, degrees.
function Scan:elevation(row)
  local cfg = self.config
  local lo, hi = cfg.el_min, cfg.el_max
  if self.rows == 1 then return lo end
  return lo + row * (hi - lo) / (self.rows - 1)
end

--- range, row, column of the closest hit, or nil.
function Scan:nearest()
  local best, br, bc
  for i, r in ipairs(self.ranges) do
    if r ~= M.INF and (best == nil or r < best) then
      best, br, bc = r, math.floor((i - 1) / self.columns), (i - 1) % self.columns
    end
  end
  if best == nil then return nil end
  return best, br, bc
end

-- ---------------------------------------------------------------- Lidar

local Lidar = {}
Lidar.__index = Lidar
Lidar.__tostring = function(l)
  return string.format("<lidar '%s' on %s>", l.name, l.hub.peripheral.name)
end

local function new_lidar(hub, name)
  return setmetatable({ hub = hub, name = name }, Lidar)
end

function Lidar:_call(method, ...)
  return self.hub.peripheral.call(method, self.name, ...)
end

--- Where the sensor sits: {x, y, z, yaw, mount, block, facing} (computer frame).
function Lidar:mount() return self:_call("mount") end

--- Set any of az_min, az_max, az_steps, el_min, el_max, rows, range; returns the full setting.
function Lidar:configure(settings) return self:_call("configure", settings or {}) end

function Lidar:config() return self:_call("get_config") end

--- Scan continuously (survives reloads and moves).
function Lidar:start() self:_call("start") end

function Lidar:stop() self:_call("stop") end

--- Sequence number of the latest finished scan (0 = none yet).
function Lidar:seq() return self:_call("get_seq") end

--- The latest finished scan, or nil.
function Lidar:latest()
  local info = self:_call("get_scan")
  if info == nil then return nil end
  return new_scan(info)
end

--- Wait for a scan with seq > `after` and return it (nil after `timeout` seconds).
function Lidar:wait(after, timeout)
  after = after or 0
  local waited = 0
  while self:seq() <= after do
    if timeout ~= nil and waited >= timeout then return nil end
    peripheral.pull_event("lidar_scan", 0.25)
    waited = waited + 0.25
  end
  return self:latest()
end

--- Take one scan and return it (nil on timeout, 10 s by default).
function Lidar:scan(timeout)
  local target = self:_call("scan")
  return self:wait(target - 1, timeout == nil and 10 or timeout)
end

--- Hits of the latest scan as {x, y, z} lists in the computer frame.
function Lidar:points()
  local info = self:_call("get_points")
  if info == nil then return {} end
  local f = M.unpack_f32(info.points)
  local out = {}
  for i = 1, #f - 2, 3 do out[#out + 1] = { f[i], f[i + 1], f[i + 2] } end
  return out
end

-- ---------------------------------------------------------------- Hub

local Hub = {}
Hub.__index = Hub
Hub.__tostring = function(h)
  return string.format("<wired sensors on %s: %s>", h.peripheral.name, table.concat(h:names(), ", "))
end

local function new_hub(p)
  return setmetatable({ peripheral = p }, Hub)
end

function Hub:names() return self.peripheral.names() end

--- { {name, type, mount, running}, ... }
function Hub:list() return self.peripheral.list() end

function Hub:lidar(name)
  local names = self:names()
  for _, n in ipairs(names) do
    if n == name then return new_lidar(self, name) end
  end
  error(string.format("no sensor named '%s' (have %s)", name, table.concat(names, ", ")), 2)
end

function Hub:lidars()
  local out = {}
  for _, s in ipairs(self:list()) do
    if s.type == "lidar" then out[#out + 1] = new_lidar(self, s.name) end
  end
  return out
end

--- The first Wired Sensor Module, or nil.
function M.find()
  local p = peripheral.find("wired_sensors")
  if p == nil then return nil end
  return new_hub(p)
end

function M.find_all()
  local out = {}
  for i, p in ipairs(peripheral.find_all("wired_sensors")) do out[i] = new_hub(p) end
  return out
end

return M
