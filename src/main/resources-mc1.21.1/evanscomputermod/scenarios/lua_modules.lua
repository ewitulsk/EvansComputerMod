-- lua_modules.lua: the modules Python has, from Lua.
-- Usage: lua lua_modules.lua (needs lib/tones.lua and a Speaker next to the computer)
local tones = require("lib.tones")
print("REQUIRE " .. tones.name)
print("MISSING " .. tostring((pcall(require, "lib.nothere"))))
print("TERMINAL " .. tostring(terminal == shell))

-- net: this computer's interfaces and DNS server
local ifs = net.interfaces()
print("NET " .. #ifs .. " " .. net.dns_get())

-- peripheral, sensors, controller: what is attached
local spk = peripheral.find("speaker")
print("PERIPHERAL " .. (spk and (spk.name .. "=" .. spk.type) or "none"))
print("SENSORS " .. tostring(sensors.find()) .. " CONTROLLER " .. tostring(controller.find()))

-- audio: three notes on the Speaker
local a = audio.open()
a:notes(tones.melody, 0.15)
print("AUDIO " .. a:status().rate)
a:close()

-- radio: an AX.25 packet through an AFSK modem and FM, and back
local got
radio.Flowgraph(radio.frames_source({ radio.ax25("CQ", "N0LUA", tones.name) }) .. radio.afsk1200()
  .. radio.fm_mod(3e3) .. radio.fm_demod(3e3) .. radio.afsk1200()
  .. radio.frames{ callback = function(f) got = radio.parse_ax25(f) end, echo = false }):run()
print("RADIO " .. (got and (got.src .. ">" .. got.dest .. ":" .. got.text) or "nothing"))

-- ui: without the GUI running, an app can't connect
local ok, err = pcall(ui.App, "dev.lua.console")
print("UI " .. tostring(ok) .. " " .. (tostring(err):find("console mode") and "console mode" or tostring(err)))
