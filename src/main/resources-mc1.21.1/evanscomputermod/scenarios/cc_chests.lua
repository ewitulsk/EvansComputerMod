-- cc_chests.lua: a ComputerCraft storage program: two chests next to the
-- computer, items listed and moved between them (CC's inventory methods).
local left = peripheral.wrap("left")
local right = peripheral.find("inventory", function(name) return name == "right" end)
print("TYPES " .. peripheral.getType("left") .. " " .. peripheral.getType(right))
print("SIZE " .. left.size())

local diamonds, cobble
for slot = 1, left.size() do
  local item = left.list()[slot]
  if item then
    print(("ITEM %d %s %d"):format(slot, item.name, item.count))
    if item.name == "minecraft:diamond" then diamonds = slot end
    if item.name == "minecraft:cobblestone" then cobble = slot end
  end
end

local d = left.getItemDetail(diamonds)
print("DETAIL " .. d.displayName .. " " .. d.maxCount)
print("PUSHED " .. left.pushItems("right", diamonds, 3))
print("PULLED " .. right.pullItems("left", cobble))

local function total(inv)
  local n = 0
  for _, item in pairs(inv.list()) do n = n + item.count end
  return n
end
print(("COUNTS %d %d"):format(total(left), total(right)))
print("NOTARGET " .. select(2, pcall(left.pushItems, "top", diamonds)))
print("BADSLOT " .. select(2, pcall(left.getItemDetail, 99)))
