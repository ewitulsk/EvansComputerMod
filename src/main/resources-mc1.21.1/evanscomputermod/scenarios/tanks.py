# tanks.py: two cauldrons next to the computer, as fluid_storage peripherals.
import peripheral

left = peripheral.wrap("left")
right = peripheral.wrap("right")
print("PY TYPES %s %s" % (left.type, right.type))


def show(name, p):
    t = p.tanks()
    print("PY %s %s" % (name, " ".join("%s=%d" % (f["name"], f["amount"]) for f in t) or "empty"))


show("LEFT", left)
show("RIGHT", right)
print("PY MOVED %d" % left.push_fluid("right"))
show("LEFT", left)
show("RIGHT", right)
try:
    left.push_fluid("top")
except peripheral.PeripheralError as e:
    print("PY NOTARGET %s" % e)
