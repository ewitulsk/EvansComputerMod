-- cc_world.lua: a ComputerCraft program using files, textutils, redstone,
-- parallel and peripherals, run unchanged on an ECM computer.
local state = { name = "base", count = 3, tags = { "a", "b" } }
fs.makeDir("data")
local h = fs.open("data/state.txt", "w")
h.write(textutils.serialize(state))
h.close()

h = fs.open("data/state.txt", "r")
local back = textutils.unserialize(h.readAll())
h.close()
print("FILES " .. table.concat(fs.list("data"), ",") .. " " .. tostring(fs.getSize("data/state.txt") > 0))
print("ROUNDTRIP " .. back.name .. " " .. back.count .. " " .. #back.tags)
print("JSON " .. textutils.serializeJSON(back.tags))
print("MISSING " .. select(2, fs.open("nope.txt", "r")))
print("SIDES " .. #rs.getSides() .. " PERIPHERALS " .. #peripheral.getNames())

-- Wait for the lever on the left.
print("LEVER " .. tostring(rs.getInput("left")))
print("Flip the lever")
os.pullEvent("redstone")
print("LEVER " .. tostring(rs.getInput("left")))

-- Blink the lamp on top until a key is typed.
local blinks = 0
local winner = parallel.waitForAny(
  function()
    while true do
      rs.setOutput("top", true) sleep(0.2)
      rs.setOutput("top", false) sleep(0.2)
      blinks = blinks + 1
    end
  end,
  function()
    print("Press K")
    os.pullEvent("char")
  end)
print("WINNER " .. winner .. " BLINKED " .. tostring(blinks > 0))
rs.setOutput("top", true)
print("LAMP " .. tostring(rs.getOutput("top")))
