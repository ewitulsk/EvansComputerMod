# console_demo.py: the console modules Lua's CC globals are built on, from Python.
import term, colors, events, keys, computer

print("PY CONSOLE starting")
events.sleep(1)

term.set_background_color(colors.black)
term.clear()
row = 0


def line(text, colour=colors.white):
    global row
    row += 1
    term.set_cursor_pos(1, row)
    term.set_text_color(colour)
    term.write(text)


w, h = term.get_size()
line("PY TERM %dx%d" % (w, h), colors.lime)
line("PY CLOCK day %d hour %.2f id %d" % (computer.day(), computer.time(), computer.id()))

t = events.start_timer(0.3)
events.queue("note", "from python")
ev = events.pull()
line("PY EVENT %s %s" % ev)
ev = events.pull("timer")
line("PY TIMER %s" % (ev[1] == t))

line("PY press Q")
while True:
    ev = events.pull("key")
    line("PY KEY %s" % keys.get_name(ev[1]))
    if ev[1] == keys.q:
        break

term.set_cursor_pos(1, h)
term.write_wrapped("PY DONE\n")
