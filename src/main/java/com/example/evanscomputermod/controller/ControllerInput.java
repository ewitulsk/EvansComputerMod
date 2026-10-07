package com.example.evanscomputermod.controller;

import org.jetbrains.annotations.Nullable;

/**
 * Every bindable input of the Wireless Xbox Controller: the 15 buttons and the
 * four directions of each stick. A program never sees keys, only these
 * inputs; the controller item maps keyboard keys onto them.
 *
 * <p>Key names are Minecraft's ({@code InputConstants}), e.g.
 * {@code key.keyboard.j}, so a binding means the same key on any keyboard
 * layout and on any computer.
 */
public enum ControllerInput {
    A("a", "A", Button.A, "key.keyboard.j"),
    B("b", "B", Button.B, "key.keyboard.k"),
    X("x", "X", Button.X, "key.keyboard.u"),
    Y("y", "Y", Button.Y, "key.keyboard.i"),
    LB("lb", "LB", Button.LB, "key.keyboard.q"),
    RB("rb", "RB", Button.RB, "key.keyboard.e"),
    LT("lt", "LT", null, "key.keyboard.1"),
    RT("rt", "RT", null, "key.keyboard.3"),
    BACK("back", "Back", Button.BACK, "key.keyboard.backspace"),
    START("start", "Start", Button.START, "key.keyboard.enter"),
    GUIDE("guide", "Guide", Button.GUIDE, ""),
    LS("ls", "Left stick click", Button.LS, "key.keyboard.left.shift"),
    RS("rs", "Right stick click", Button.RS, ""),
    DPAD_UP("dpad_up", "D-pad up", Button.DPAD_UP, "key.keyboard.up"),
    DPAD_DOWN("dpad_down", "D-pad down", Button.DPAD_DOWN, "key.keyboard.down"),
    DPAD_LEFT("dpad_left", "D-pad left", Button.DPAD_LEFT, "key.keyboard.left"),
    DPAD_RIGHT("dpad_right", "D-pad right", Button.DPAD_RIGHT, "key.keyboard.right"),
    LS_UP("ls_up", "Left stick up", null, "key.keyboard.w"),
    LS_DOWN("ls_down", "Left stick down", null, "key.keyboard.s"),
    LS_LEFT("ls_left", "Left stick left", null, "key.keyboard.a"),
    LS_RIGHT("ls_right", "Left stick right", null, "key.keyboard.d"),
    RS_UP("rs_up", "Right stick up", null, "key.keyboard.keypad.8"),
    RS_DOWN("rs_down", "Right stick down", null, "key.keyboard.keypad.5"),
    RS_LEFT("rs_left", "Right stick left", null, "key.keyboard.keypad.4"),
    RS_RIGHT("rs_right", "Right stick right", null, "key.keyboard.keypad.6");

    /** The digital buttons, with the bit each has in a state's button mask. */
    public enum Button {
        A, B, X, Y, LB, RB, BACK, START, GUIDE, LS, RS, DPAD_UP, DPAD_DOWN, DPAD_LEFT, DPAD_RIGHT;

        public int bit() {
            return 1 << ordinal();
        }

        /** Lower-case name programs use ({@code "dpad_up"}). */
        public String id() {
            return name().toLowerCase(java.util.Locale.ROOT);
        }

        @Nullable
        public static Button byId(String id) {
            for (Button b : values()) {
                if (b.id().equals(id)) return b;
            }
            return null;
        }
    }

    private final String id;
    private final String label;
    @Nullable private final Button button;
    private final String defaultKey;

    ControllerInput(String id, String label, @Nullable Button button, String defaultKey) {
        this.id = id;
        this.label = label;
        this.button = button;
        this.defaultKey = defaultKey;
    }

    /** Stable id stored in the item's NBT. */
    public String id() { return id; }

    public String label() { return label; }

    /** The digital button this input presses, or null for triggers and stick directions. */
    @Nullable public Button button() { return button; }

    /** Default key name, or "" for unbound. */
    public String defaultKey() { return defaultKey; }

    @Nullable
    public static ControllerInput byId(String id) {
        for (ControllerInput in : values()) {
            if (in.id.equals(id)) return in;
        }
        return null;
    }
}
