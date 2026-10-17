# console_world.py: files, textutils and redstone from Python (the modules Lua's CC globals use).
import fs, textutils, redstone, events

state = {"name": "base", "count": 3, "tags": ["a", "b"]}
fs.make_dir("pydata")
h = fs.open("pydata/state.txt", "w")
h.write(textutils.serialize(state))
h.close()
h = fs.open("pydata/state.txt", "r")
back = textutils.unserialize(h.read_all())
h.close()
print("PY FILES %s" % ",".join(fs.list("pydata")))
print("PY ROUNDTRIP %s %d %d" % (back["name"], back["count"], len(back["tags"])))
print("PY JSON %s" % textutils.serialize_json(back["tags"]))
print("PY MISSING %s" % fs.open("nope.txt", "r")[1])
print("PY LEVER %s" % redstone.get_input("left"))
print("PY flip the lever")
events.pull("redstone")
print("PY LEVER %s" % redstone.get_input("left"))
redstone.set_output("top", True)
print("PY LAMP %s" % redstone.get_output("top"))
