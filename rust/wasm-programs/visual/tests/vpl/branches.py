import terminal

_block2_result = terminal.input("Continue? ")
_block3_result = _block2_result == "yes"
if _block3_result:
    terminal.println("Going on")
    if _block2_result == 'yes':
        terminal.println(3.5)
    else:
        terminal.sleep(1.0)
elif len(_block2_result) > 3:
    terminal.println("Long \"answer\"")
    terminal.println(3.5)
else:
    terminal.println("C:\\path")
