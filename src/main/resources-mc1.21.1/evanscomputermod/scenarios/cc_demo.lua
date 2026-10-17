-- cc_demo.lua: a ComputerCraft program, written the way CC programs are,
-- running unchanged on an ECM computer (no require, no flags).
print("CC DEMO starting")
sleep(1)

term.setBackgroundColour(colours.black)
term.clear()
term.setCursorPos(1, 1)
term.setTextColour(colours.yellow)
print("CC DEMO " .. os.version())
term.setTextColour(colours.white)
local w, h = term.getSize()
print(("SIZE %dx%d"):format(w, h))
print(("CLOCK day %d hour %.2f id %d"):format(os.day(), os.time(), os.getComputerID()))
print("BADLOCALE " .. tostring((pcall(os.time, "mars"))))

-- One event queue: our own event first, then the timer.
local timer = os.startTimer(0.5)
local start = os.clock()
os.queueEvent("note", "queued first")
local ev, text = os.pullEvent()
print("EVENT " .. ev .. " " .. text)
local _, id = os.pullEvent("timer")
print(("TIMER %s %.1f"):format(tostring(id == timer), os.clock() - start))

-- read() edits the line as it is typed.
write("Name? ")
local name = read()
print("HELLO " .. name .. "!")

-- Keys come with their key codes; only Q stops.
print("Press Q")
while true do
  local _, key = os.pullEvent("key")
  print("KEY " .. (keys.getName(key) or tostring(key)))
  if key == keys.q then break end
end

term.setCursorPos(1, h)
term.blit("DONE", "e5b0", "ffff")
print()
