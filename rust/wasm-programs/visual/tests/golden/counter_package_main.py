import ui

def _here(name):
    try:
        import os
        return os.path.join(os.path.dirname(os.path.abspath(__file__)), name)
    except Exception:
        return name

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
