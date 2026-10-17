-- ComputerCraft's `colours`: `colors` with British spellings (grey, lightGrey).
local colors = require("colors")
local M = {}
for k, v in pairs(colors) do M[k] = v end
M.grey, M.lightGrey, M.light_grey = colors.gray, colors.lightGray, colors.lightGray
M.gray, M.lightGray, M.light_gray = nil, nil, nil
return M
