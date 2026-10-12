package com.example.evanscomputermod.computer.ui;

/**
 * Keyboard codes for the UI protocol (§7): keys travel as USB HID usages
 * (page 7), so programs never see GLFW or the player's layout. GLFW key
 * tokens name physical keys after the US layout, as HID usages do, so the
 * mapping is a fixed table. GLFW's modifier bits (shift 1, ctrl 2, alt 4,
 * super 8) are already the protocol's {@code Mods}.
 */
public final class HidKeys {
    private HidKeys() {}

    public static final int ENTER = 0x28, ESCAPE = 0x29, BACKSPACE = 0x2A, TAB = 0x2B;
    public static final int MODS_MASK = 0xF;

    /** The HID usage of a GLFW key code, or 0 if it has none. */
    public static int fromGlfw(int k) {
        if (k >= 65 && k <= 90) return 0x04 + (k - 65);          // A-Z
        if (k >= 49 && k <= 57) return 0x1E + (k - 49);          // 1-9
        if (k >= 290 && k <= 301) return 0x3A + (k - 290);       // F1-F12
        if (k >= 302 && k <= 313) return 0x68 + (k - 302);       // F13-F24
        if (k >= 321 && k <= 329) return 0x59 + (k - 321);       // keypad 1-9
        return switch (k) {
            case 48 -> 0x27;   // 0
            case 257 -> ENTER;
            case 256 -> ESCAPE;
            case 259 -> BACKSPACE;
            case 258 -> TAB;
            case 32 -> 0x2C;   // space
            case 45 -> 0x2D;   // -
            case 61 -> 0x2E;   // =
            case 91 -> 0x2F;   // [
            case 93 -> 0x30;   // ]
            case 92 -> 0x31;   // backslash
            case 59 -> 0x33;   // ;
            case 39 -> 0x34;   // '
            case 96 -> 0x35;   // `
            case 44 -> 0x36;   // ,
            case 46 -> 0x37;   // .
            case 47 -> 0x38;   // /
            case 280 -> 0x39;  // caps lock
            case 283 -> 0x46;  // print screen
            case 281 -> 0x47;  // scroll lock
            case 284 -> 0x48;  // pause
            case 260 -> 0x49;  // insert
            case 268 -> 0x4A;  // home
            case 266 -> 0x4B;  // page up
            case 261 -> 0x4C;  // delete
            case 269 -> 0x4D;  // end
            case 267 -> 0x4E;  // page down
            case 262 -> 0x4F;  // right
            case 263 -> 0x50;  // left
            case 264 -> 0x51;  // down
            case 265 -> 0x52;  // up
            case 282 -> 0x53;  // num lock
            case 331 -> 0x54;  // keypad /
            case 332 -> 0x55;  // keypad *
            case 333 -> 0x56;  // keypad -
            case 334 -> 0x57;  // keypad +
            case 335 -> 0x58;  // keypad enter
            case 320 -> 0x62;  // keypad 0
            case 330 -> 0x63;  // keypad .
            case 161, 162 -> 0x64; // non-US \ and #
            case 348 -> 0x65;  // menu
            case 336 -> 0x67;  // keypad =
            case 341 -> 0xE0;  // left ctrl
            case 340 -> 0xE1;  // left shift
            case 342 -> 0xE2;  // left alt
            case 343 -> 0xE3;  // left super
            case 345 -> 0xE4;  // right ctrl
            case 344 -> 0xE5;  // right shift
            case 346 -> 0xE6;  // right alt
            case 347 -> 0xE7;  // right super
            default -> 0;
        };
    }

    /** Is this usage a modifier key (left/right ctrl, shift, alt, super)? */
    public static boolean isModifier(int hid) {
        return hid >= 0xE0 && hid <= 0xE7;
    }
}
