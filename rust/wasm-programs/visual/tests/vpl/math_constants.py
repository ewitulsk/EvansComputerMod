import terminal

_block4_result = 7 * 3
_block5_result = 7 ** 2
_block6_result = -_block5_result
_block7_result = _block4_result - _block6_result
_block8_result = _block7_result / 3
_block10_result = int(_block8_result)
_block12_result = str(_block10_result)
_block14_result = str("Result: ") + str(_block12_result)
terminal.println(_block14_result)
_block16_result = _block10_result > 10
if _block16_result:
    terminal.println("big")
else:
    _block9_result = _block8_result % 4
    _block11_result = float(_block9_result)
    _block20_result = 2 <= _block11_result
    _block21_result = _block20_result != "x"
    terminal.println(_block21_result)
    _block22_result = 3 < "q"
    _block23_result = _block22_result >= 0
    terminal.println(_block23_result)
