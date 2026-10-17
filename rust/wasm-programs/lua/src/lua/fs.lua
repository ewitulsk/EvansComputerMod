--[[ The computer's files, by path from its root ("progs/a.lua" and
"/progs/a.lua" are the same file). The Lua counterpart of Python's `fs`
module (same functions), with ComputerCraft's names too.

    fs.list("/")                         -- sorted names
    fs.make_dir("data")                  -- fs.makeDir
    local h = fs.open("data/log.txt", "a")
    h.writeLine("hello") h.close()
    for _, p in ipairs(fs.find("progs/*.lua")) do print(p, fs.get_size(p)) end
    fs.combine("a", "../b")              -- "b"

fs.open(path, mode) gives a handle (nil and a message if it can't):
  "r"  readLine(keep_newline), readAll(), read(n), close()
  "w"/"a"  write(text), writeLine(text), flush(), close()
  "rb"/"wb"/"ab"  read() a byte as a number, read(n), write(byte or text), seek(whence, offset)
Errors are CC's: "/x: No such file", "/x: Not a directory", "/x: File exists".
]]

local c = require("_console")

local M = {}

M.list = c.fs_list
M.exists = c.fs_exists
M.is_dir = c.fs_is_dir
M.is_read_only = c.fs_is_read_only
M.get_size = c.fs_get_size
M.make_dir = c.fs_make_dir
M.delete = c.fs_delete
M.copy = c.fs_copy
M.move = c.fs_move
M.find = c.fs_find
M.combine = c.fs_combine
M.get_name = c.fs_get_name
M.get_dir = c.fs_get_dir
M.attributes = c.fs_attributes

--- Free bytes on the drive, or "unlimited".
function M.get_free_space(path)
  return c.fs_get_free_space(path or "") or "unlimited"
end

--- The drive a path is on ("hdd"), or nil if it doesn't exist.
function M.get_drive(path)
  if not c.fs_exists(path) then return nil end
  return "hdd"
end

--- The drive's size in bytes, or nil (the computer's storage has no fixed size here).
function M.get_capacity(path)
  if not c.fs_exists(path) then error("/" .. c.fs_combine(path) .. ": No such file", 2) end
  return nil
end

function M.is_drive_root(path)
  return c.fs_combine(path) == ""
end

local function handle(f, mode)
  local binary = mode:find("b") ~= nil
  local h = {}
  local open = true
  local function check()
    if not open then error("attempt to use a closed file", 3) end
  end
  if mode:sub(1, 1) == "r" then
    function h.readLine(keep)
      check()
      local line = f:read(keep and "*L" or "*l")
      return line
    end
    function h.readAll()
      check()
      local s = f:read("*a")
      if s == "" and f:seek("cur") > 0 then return nil end
      return s
    end
    function h.read(n)
      check()
      if binary and n == nil then
        local b = f:read(1)
        return b and b:byte() or nil
      end
      return f:read(n or 1)
    end
  else
    function h.write(v)
      check()
      if binary and type(v) == "number" then
        f:write(string.char(v % 256))
      else
        f:write(tostring(v))
      end
    end
    function h.writeLine(v)
      check()
      f:write(tostring(v), "\n")
    end
    function h.flush()
      check()
      f:flush()
    end
  end
  if binary then
    function h.seek(whence, offset)
      check()
      return f:seek(whence or "cur", offset or 0)
    end
  end
  function h.close()
    if open then
      open = false
      f:close()
    end
  end
  h.read_line, h.read_all, h.write_line = h.readLine, h.readAll, h.writeLine
  return h
end

--- Open a file: a handle, or nil and why not.
function M.open(path, mode)
  if type(mode) ~= "string" or not ({ r = 1, w = 1, a = 1, rb = 1, wb = 1, ab = 1 })[mode] then
    error("Unsupported mode", 2)
  end
  local name = "/" .. c.fs_combine(path)
  if name:sub(1, 3) == "/.." then return nil, name .. ": Invalid Path" end
  local real = c.fs_real_path(path)
  if c.fs_is_dir(path) then return nil, name .. ": Is a directory" end
  if mode:sub(1, 1) == "r" and not c.fs_exists(path) then return nil, name .. ": No such file" end
  if mode:sub(1, 1) ~= "r" then
    local dir = c.fs_get_dir(path)
    if dir ~= "" and dir ~= ".." then c.fs_make_dir(dir) end
  end
  local f, e = io.open(real, mode:sub(1, 1) .. "b")
  if not f then return nil, name .. ": " .. tostring(e) end
  return handle(f, mode)
end

M.isDir, M.isReadOnly, M.getSize, M.makeDir = M.is_dir, M.is_read_only, M.get_size, M.make_dir
M.getName, M.getDir, M.getFreeSpace, M.getDrive = M.get_name, M.get_dir, M.get_free_space, M.get_drive
M.getCapacity, M.isDriveRoot = M.get_capacity, M.is_drive_root

return M
