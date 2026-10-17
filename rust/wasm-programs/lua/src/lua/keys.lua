--[[ Key codes, as "key" and "key_up" events give them (the GLFW codes
ComputerCraft uses). The Lua counterpart of Python's `keys` module.

    local _, code = events.pull("key")
    if code == keys.enter then ... end
    keys.get_name(code)    -- "enter" (keys.getName)

Names: a-z, zero-nine, f1-f25, numPad0-numPad9, space, enter, tab,
backspace, delete, insert, up, down, left, right, home, ["end"], pageUp,
pageDown, leftShift, rightShift, leftCtrl, rightCtrl, leftAlt, rightAlt, ...
]]

local M = {}
local by_code = {}
for name, code in pairs(require("_console").key_names()) do
  M[name] = code
  by_code[code] = name
end

--- The code of key `name` (keys.get("end") is keys["end"]).
function M.get(name)
  return rawget(M, name)
end

--- The name of a key code, or nil.
function M.get_name(code)
  return by_code[code]
end
M.getName = M.get_name

return M
