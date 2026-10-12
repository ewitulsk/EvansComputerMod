import terminal
import peripheral

(peripheral.wrap("left") or peripheral.find("test_lamp")).set_color(255)
_block3_result = (peripheral.wrap("") or peripheral.find("test_lamp")).get_level()
terminal.println(_block3_result)
(peripheral.wrap("") or peripheral.find("test_lamp")).set_label("hi", True)
terminal.set_redstone(5, 7)
