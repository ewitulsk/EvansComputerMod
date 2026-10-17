-- lua_counter.lua: a GUI app in Lua (the ui module): a button that counts.
-- Usage: gui launch lua /lua_counter.lua
local app = ui.App("dev.lua.counter")
local count = 0
local label = ui.Label("Count: 0", { id = "count", style = "title" })
app:window("Lua Counter", ui.Column{ gap = 8, padding = 12, children = {
  label,
  ui.Button("Add one", { id = "add", style = "primary", on_click = function()
    count = count + 1
    label.text = "Count: " .. count
  end }),
} })
app:run()
