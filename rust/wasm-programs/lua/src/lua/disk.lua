--[[ Disks in Disk Drives, by the drive's attachment name ("left", "top" ...).
The Lua counterpart of Python's `disk` module (same functions), with
ComputerCraft's names too. A floppy in a drive next to the computer is the
folder disk/ (disk2/ ... for more drives).

    if disk.is_present("left") then           -- disk.isPresent
      print(disk.get_mount_path("left"))      -- "disk"
      print(disk.get_label("left"), disk.get_id("left"))
      disk.set_label("left", "backups")
      disk.eject("left")
    end

Events: "disk", side when a floppy goes in; "disk_eject", side when it comes out.
]]

local M = {}

local function drive(name)
  if type(name) ~= "string" then error("bad argument #1 (string expected, got " .. type(name) .. ")", 3) end
  return peripheral.get_type(name) == "drive"
end

local function call(name, method, ...)
  if not drive(name) then return nil end
  return peripheral.call(name, method, ...)
end

function M.is_present(name) return drive(name) and call(name, "is_disk_present") or false end
function M.has_data(name) return drive(name) and call(name, "has_data") or false end
function M.get_mount_path(name) return call(name, "get_mount_path") end
function M.get_label(name) return call(name, "get_disk_label") end
function M.set_label(name, label) if drive(name) then call(name, "set_disk_label", label) end end
function M.get_id(name) return call(name, "get_disk_id") end
function M.eject(name) if drive(name) then call(name, "eject_disk") end end
-- Music discs need a jukebox drive: none here.
function M.has_audio(name) drive(name) return false end
function M.get_audio_title(name) drive(name) return nil end
function M.play_audio(name) drive(name) end
function M.stop_audio(name) drive(name) end

M.isPresent, M.hasData, M.getMountPath, M.getLabel = M.is_present, M.has_data, M.get_mount_path, M.get_label
M.setLabel, M.getID, M.hasAudio, M.getAudioTitle = M.set_label, M.get_id, M.has_audio, M.get_audio_title
M.playAudio, M.stopAudio = M.play_audio, M.stop_audio

return M
