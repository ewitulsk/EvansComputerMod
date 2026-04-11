"""Comprehensive test suite for the framebuffer graphics library.

Tests all fb_* host functions via the Python display module.
Prints PASS/FAIL for each test. Returns exit code 0 if all pass.
"""
import display
import terminal

errors = []
test_count = 0

def test(name, condition):
    global test_count
    test_count += 1
    if condition:
        terminal.println("  PASS: " + name)
    else:
        terminal.println("  FAIL: " + name)
        errors.append(name)

def run_tests():
    terminal.println("=== Framebuffer Graphics Library Tests ===")
    terminal.println("")

    # ----- Test 1: Display attachment -----
    terminal.println("[1] Display attachment")
    w = display.get_width()
    h = display.get_height()
    test("get_width returns positive", w > 0)
    test("get_height returns positive", h > 0)
    test("is_attached returns True", display.is_attached())
    test("width is 64", w == 64)
    test("height is 64", h == 64)
    terminal.println("")

    # ----- Test 2: Clear -----
    terminal.println("[2] Clear")
    # Initial buffer is black (0,0,0,255), so clear to a different color
    display.clear(255, 0, 0)
    tiles = display.flush()
    test("clear to red + flush returns tiles > 0", tiles > 0)

    # Flush again without changes should return 0
    tiles2 = display.flush()
    test("flush with no changes returns 0", tiles2 == 0)
    terminal.println("")

    # ----- Test 3: Set pixel -----
    terminal.println("[3] Set pixel")
    display.clear(0, 0, 0)
    display.flush()  # reset snapshot

    display.set_pixel(0, 0, 255, 0, 0)
    tiles = display.flush()
    test("set_pixel single pixel flushes 1 tile", tiles == 1)

    # Set pixel at different tile
    display.set_pixel(32, 32, 0, 255, 0)
    tiles = display.flush()
    test("set_pixel in different tile flushes 1 tile", tiles == 1)

    # Set pixels in same tile
    display.set_pixel(0, 0, 255, 255, 0)
    display.set_pixel(1, 1, 255, 255, 0)
    tiles = display.flush()
    test("2 pixels in same tile flushes 1 tile", tiles == 1)

    # Set pixels across 2 tiles
    display.set_pixel(0, 0, 0, 0, 255)
    display.set_pixel(16, 0, 0, 0, 255)
    tiles = display.flush()
    test("pixels in 2 tiles flushes 2 tiles", tiles == 2)
    terminal.println("")

    # ----- Test 4: Fill rect -----
    terminal.println("[4] Fill rect")
    display.clear(0, 0, 0)
    display.flush()

    # Small rect in one tile
    display.fill_rect(0, 0, 8, 8, 255, 0, 0)
    tiles = display.flush()
    test("small rect in 1 tile flushes 1 tile", tiles == 1)

    # Rect spanning 4 tiles (crossing 16px boundaries)
    display.fill_rect(8, 8, 16, 16, 0, 255, 0)
    tiles = display.flush()
    test("rect spanning 4 tiles flushes 4 tiles", tiles == 4)

    # Full display rect
    display.fill_rect(0, 0, 64, 64, 0, 0, 255)
    tiles = display.flush()
    test("full display fill flushes all tiles", tiles == 16)  # 64/16 = 4x4 = 16
    terminal.println("")

    # ----- Test 5: Fill rect large -----
    terminal.println("[5] Fill rect - large area")
    display.clear(0, 0, 0)
    display.flush()

    display.fill_rect(0, 0, 48, 48, 255, 0, 0)
    tiles = display.flush()
    test("large fill_rect flushes multiple tiles", tiles > 4)
    terminal.println("")

    # ----- Test 6: Rect outline -----
    terminal.println("[6] Rect outline")
    display.clear(0, 0, 0)
    display.flush()

    display.rect(4, 4, 24, 24, 255, 255, 0)
    tiles = display.flush()
    test("rect outline flushes tiles", tiles > 0)
    terminal.println("")

    # ----- Test 7: Lines -----
    terminal.println("[7] Lines")
    display.clear(0, 0, 0)
    display.flush()

    display.hline(0, 32, 64, 255, 0, 255)
    tiles = display.flush()
    test("hline flushes", tiles > 0)

    display.vline(32, 0, 64, 0, 255, 255)
    tiles = display.flush()
    test("vline flushes", tiles > 0)
    terminal.println("")

    # ----- Test 8: Text rendering -----
    terminal.println("[8] Text rendering")
    display.clear(0, 0, 0)
    display.flush()

    result = display.text(0, 0, "Hello")
    test("text returns char count", result == 5)
    tiles = display.flush()
    test("text flushes tiles", tiles > 0)

    # Text with custom colors
    result = display.text_colored(0, 20, "World", 255, 0, 0)
    test("text_colored returns char count", result == 5)
    terminal.println("")

    # ----- Test 9: Idempotent operations -----
    terminal.println("[9] Idempotent operations")
    display.clear(128, 128, 128)
    display.flush()

    # Clear with same color should still mark dirty (pixels differ from snapshot)
    # but after flush, doing the same clear should not produce changes
    display.clear(128, 128, 128)
    tiles = display.flush()
    test("same clear after flush returns 0 tiles", tiles == 0)
    terminal.println("")

    # ----- Test 10: Boundary conditions -----
    terminal.println("[10] Boundary conditions")
    display.clear(0, 0, 0)
    display.flush()

    # Pixel at max coordinates
    display.set_pixel(63, 63, 255, 255, 255)
    tiles = display.flush()
    test("pixel at max coords flushes", tiles == 1)

    # Pixel out of bounds (should be silently ignored)
    display.set_pixel(-1, -1, 255, 0, 0)
    display.set_pixel(64, 64, 255, 0, 0)
    display.set_pixel(1000, 1000, 255, 0, 0)
    tiles = display.flush()
    test("out of bounds pixels cause 0 tile flushes", tiles == 0)

    # Rect partially out of bounds
    display.fill_rect(-5, -5, 20, 20, 255, 0, 0)
    tiles = display.flush()
    test("partial OOB rect still flushes", tiles > 0)

    # Rect completely out of bounds
    display.clear(0, 0, 0)
    display.flush()
    display.fill_rect(100, 100, 10, 10, 255, 0, 0)
    tiles = display.flush()
    test("fully OOB rect flushes 0 tiles", tiles == 0)
    terminal.println("")

    # ----- Test 11: Multiple operations before flush -----
    terminal.println("[11] Batched operations")
    display.clear(0, 0, 0)
    display.flush()

    display.fill_rect(0, 0, 32, 32, 255, 0, 0)
    display.fill_rect(32, 0, 32, 32, 0, 255, 0)
    display.fill_rect(0, 32, 32, 32, 0, 0, 255)
    display.fill_rect(32, 32, 32, 32, 255, 255, 0)
    tiles = display.flush()
    test("4 quadrant fills flush all 16 tiles", tiles == 16)
    terminal.println("")

    # ----- Test 12: Overwrite previous drawing -----
    terminal.println("[12] Overwrite")
    display.clear(255, 0, 0)  # all red
    display.flush()

    display.clear(255, 0, 0)  # same color
    tiles = display.flush()
    test("overwrite same color = 0 tiles", tiles == 0)

    display.fill_rect(0, 0, 16, 16, 0, 255, 0)  # green in top-left
    display.fill_rect(0, 0, 16, 16, 255, 0, 0)  # back to red
    tiles = display.flush()
    test("overwrite back to original = 0 tiles", tiles == 0)
    terminal.println("")

    # ----- Summary -----
    terminal.println("=== Results ===")
    passed = test_count - len(errors)
    terminal.println(str(passed) + "/" + str(test_count) + " tests passed")
    if errors:
        terminal.println("FAILED tests:")
        for e in errors:
            terminal.println("  - " + e)
    else:
        terminal.println("ALL TESTS PASSED")

run_tests()
