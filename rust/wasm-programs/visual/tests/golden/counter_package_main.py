import ui

def _here(name):
    # Next to this script (a packaged app keeps main.ui.json beside main.py).
    try:
        d = __file__.rsplit("/", 1)[0] if "/" in __file__ else ""
    except NameError:
        d = ""
    return d + "/" + name if d else name

_app = None
_layout = None

def _on_add_click_1(e):
    global count
    _block1_value = e.value
    _block6_result = count + 1
    count = _block6_result
    _block7_result = str(count)
    _block8_result = str("Count: ") + str(_block7_result)
    _layout.set("count", text=str(_block8_result))

_app, _win = ui.load(_here("main.ui.json"), {"add_click": _on_add_click_1}, app_id="dev.visual.counter")
_layout = _app.layout(_win)
count = 0
_app.run()
