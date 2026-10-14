import terminal

def _on_key_1(data):
    _block1_key = data['key']
    _block3_result = str("Key: ") + str(_block1_key)
    terminal.println(_block3_result)
terminal.on_interrupt(terminal.IRQ_KEYBOARD, _on_key_1)
def _on_redstone_4(data):
    _block4_sides = data['sides']
    terminal.println(_block4_sides)
terminal.on_interrupt(terminal.IRQ_REDSTONE, _on_redstone_4)
terminal.println("Waiting for events")
while True:
    terminal.sleep(1.0)
