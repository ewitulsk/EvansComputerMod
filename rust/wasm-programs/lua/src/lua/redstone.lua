--[[ Redstone by side name: "top", "bottom", "front", "back", "left",
"right" (left and right as you face the computer's screen). The Lua
counterpart of Python's `redstone` module (same functions); `rs` is the same
module, and ComputerCraft's names are here too.

    redstone.set_output("top", true)           -- rs.setOutput("top", true)
    redstone.get_input("left")                 -- true while a lever on the left is on
    redstone.set_analog_output("back", 7)      -- rs.setAnalogOutput / setAnalogueOutput
    redstone.get_sides()                       -- {"top", "bottom", "left", "right", "front", "back"}

Bundled cables need a mod that adds them: here bundled input is 0 and
bundled output is ignored.
]]

local c = require("_console")

local M = {}

function M.get_sides()
  return { "top", "bottom", "left", "right", "front", "back" }
end

function M.get_input(side) return c.rs_get_input(side) > 0 end
function M.get_output(side) return c.rs_get_output(side) > 0 end
function M.set_output(side, on) c.rs_set_output(side, on and 15 or 0) end

function M.get_analog_input(side) return c.rs_get_input(side) end
function M.get_analog_output(side) return c.rs_get_output(side) end
function M.set_analog_output(side, power)
  if type(power) ~= "number" or power < 0 or power > 15 then error("Expected number in range 0-15", 2) end
  c.rs_set_output(side, math.floor(power))
end

function M.get_bundled_input(side) c.rs_get_input(side) return 0 end
function M.get_bundled_output(side) c.rs_get_output(side) return 0 end
function M.set_bundled_output(side, colours) c.rs_get_output(side) end
function M.test_bundled_input(side, colour) return M.get_bundled_input(side) % (2 * colour) >= colour end

M.getSides, M.getInput, M.getOutput, M.setOutput = M.get_sides, M.get_input, M.get_output, M.set_output
M.getAnalogInput, M.getAnalogOutput, M.setAnalogOutput = M.get_analog_input, M.get_analog_output, M.set_analog_output
M.getAnalogueInput, M.getAnalogueOutput, M.setAnalogueOutput = M.get_analog_input, M.get_analog_output, M.set_analog_output
M.getBundledInput, M.getBundledOutput, M.setBundledOutput = M.get_bundled_input, M.get_bundled_output, M.set_bundled_output
M.testBundledInput = M.test_bundled_input

return M
