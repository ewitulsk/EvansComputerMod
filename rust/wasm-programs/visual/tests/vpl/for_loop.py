import terminal

for n in range(1, 4):
    _block4_result = n * 2
    terminal.println(_block4_result)
terminal.sleep(0.5)
terminal.clear()
terminal.set_redstone(2, 15)
terminal.set_redstone(3, 0)
