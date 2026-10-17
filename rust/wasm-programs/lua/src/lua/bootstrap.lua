-- Run once when the interpreter starts (lib.rs `install`), with the table of
-- Lua-source modules ({name = source}).
local sources = ...
local preload = package.preload

-- Lua 5.1 names that Lua 5.2 only keeps behind LUA_COMPAT_ALL (the vendored
-- build doesn't set it) and that most older scripts still use.
unpack = unpack or table.unpack
loadstring = loadstring or load

for name, src in pairs(sources) do
  preload[name] = function(modname)
    local f = assert(load(src, "=" .. name))
    return f(modname)
  end
end

-- require of the computer's own files, like Python's import: "a.b" is
-- a/b.lua or a/b/init.lua, next to the running script or from the computer's
-- root.
local function read(path)
  local ok, shell = pcall(require, "shell")
  if ok and type(shell) == "table" then
    return shell.read_file(path)
  end
  local f = io.open(path, "rb")
  if not f then return nil end
  local s = f:read("*a")
  f:close()
  return s
end

local function script_dir()
  local a = rawget(_G, "arg")
  local s = type(a) == "table" and a[0]
  if type(s) ~= "string" then return nil end
  return s:match("^(.*)/[^/]*$")
end

local function search(name)
  local base = name:gsub("%.", "/")
  local dirs = { "" }
  local d = script_dir()
  if d and d ~= "" then table.insert(dirs, 1, d .. "/") end
  local tried = {}
  for _, dir in ipairs(dirs) do
    for _, path in ipairs({ dir .. base .. ".lua", dir .. base .. "/init.lua" }) do
      local src = read(path)
      if src then
        local f, e = load(src, "@" .. path)
        if not f then error("error loading module '" .. name .. "' from " .. path .. ":\n\t" .. e, 0) end
        return f, path
      end
      tried[#tried + 1] = "\n\tno file '" .. path .. "' on this computer"
    end
  end
  return table.concat(tried)
end
table.insert(package.searchers, 2, search)

-- The modules are globals too, loaded on first use: `radio.tone(...)` works
-- without a require.
setmetatable(_G, {
  __index = function(t, k)
    if type(k) == "string" and preload[k] ~= nil then
      local m = require(k)
      rawset(t, k, m)
      return m
    end
  end,
})
