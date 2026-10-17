-- lua_demo.lua: a tour of the Lua interpreter on an ECM computer.
-- Usage: lua lua_demo.lua <count>
local n = tonumber(arg[1])
if not n then error("give a count, e.g. lua lua_demo.lua 5", 0) end

print("LUA " .. _VERSION)

-- Coroutines: a generator that yields squares.
local squares = coroutine.wrap(function()
  for i = 1, n do coroutine.yield(i * i) end
end)
local got = {}
for _ = 1, n do got[#got + 1] = squares() end
print("SQUARES " .. table.concat(got, " "))

-- Errors are values: pcall catches them.
local ok, err = pcall(function() error({ code = 42 }) end)
print("CAUGHT " .. tostring(ok) .. " " .. err.code)

-- Files through Lua's io library, read back through the shell module.
local f = assert(io.open("lua_out.txt", "w"))
f:write("count=" .. n)
f:close()
print("FILE " .. shell.read_file("/lua_out.txt"))

-- Peripherals attached to this computer.
local names = {}
for _, p in ipairs(peripheral.list()) do names[#names + 1] = p.name .. "=" .. p.type end
print("PERIPHERALS " .. #names .. " " .. table.concat(names, " "))

-- Redstone: power every side (the lamp on top lights up).
for side = shell.DOWN, shell.RIGHT do shell.set_redstone(side, 15) end
print("REDSTONE on")
