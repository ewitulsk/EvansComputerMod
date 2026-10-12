import ui

app = ui.App("dev.ecm.notes")
notes = app.restore().get("notes", [])
entry = ui.TextField(placeholder="New note")
rows = ui.List(items=[ui.Label(n) for n in notes])

def add(_=None):
    if entry.text:
        notes.append(entry.text)
        entry.text = ""
        rows.items = [ui.Label(n) for n in notes]

app.window("Notes", ui.Column(gap=8, padding=12, children=[
    ui.Row(gap=8, children=[entry,
        ui.Button("Add", on_click=add)]),
    rows,
]))
app.hotkey(1, "Add note", add)
app.on_suspend(lambda: {"notes": notes})
app.run()
