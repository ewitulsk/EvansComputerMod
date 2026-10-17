--[[ ComputerCraft's `parallel`: run functions side by side, each in its own
coroutine, handing every event to each one that waits for it.

    parallel.waitForAny(function() sleep(1) end, function() os.pullEvent("key") end)
    parallel.waitForAll(task1, task2)

Inside them os.pullEvent, sleep and read wait by yielding, so the others
keep running. waitForAny returns the number of the function that finished
first; an error in any of them stops all and is raised.

Lua only: Python has no coroutines to run functions this way.
]]

local M = {}

-- Coroutines run by parallel: os.pullEventRaw yields in them (lua/cc.lua).
local managed = setmetatable({}, { __mode = "k" })

--- Whether the running code is inside a function parallel runs.
function M._inside()
  local co = coroutine.running()
  return co ~= nil and managed[co] == true
end

local function run(fns, any)
  local cos, filters = {}, {}
  for i, f in ipairs(fns) do
    if type(f) ~= "function" then
      error("bad argument #" .. i .. " (function expected, got " .. type(f) .. ")", 3)
    end
    local co = coroutine.create(f)
    managed[co] = true
    cos[i] = co
  end
  local alive = #cos
  local event = { n = 0 }
  while true do
    for i = 1, #cos do
      local co = cos[i]
      if co and (filters[i] == nil or filters[i] == event[1] or event[1] == "terminate") then
        local ok, filter = coroutine.resume(co, table.unpack(event, 1, event.n))
        if not ok then error(filter, 0) end
        filters[i] = filter
        if coroutine.status(co) == "dead" then
          cos[i] = false
          alive = alive - 1
          if any then return i end
        end
      end
    end
    if alive == 0 then return nil end
    -- The next event, for every coroutine waiting (from outside: maybe a parallel itself).
    event = table.pack(os.pullEventRaw())
  end
end

function M.waitForAny(...) return run({ ... }, true) end
function M.waitForAll(...) run({ ... }, false) end
M.wait_for_any, M.wait_for_all = M.waitForAny, M.waitForAll

return M
