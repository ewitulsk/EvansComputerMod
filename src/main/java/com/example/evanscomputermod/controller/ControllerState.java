package com.example.evanscomputermod.controller;

import java.util.Set;

/**
 * One snapshot of a controller, as programs see it.
 *
 * @param buttons bit mask of {@link ControllerInput.Button#bit()}
 * @param lx left stick x, -127 (left) .. 127 (right)
 * @param ly left stick y, -127 (down) .. 127 (up)
 * @param rx right stick x
 * @param ry right stick y
 * @param lt left trigger, 0 .. 255
 * @param rt right trigger, 0 .. 255
 */
public record ControllerState(int buttons, int lx, int ly, int rx, int ry, int lt, int rt) {

    public static final ControllerState NEUTRAL = new ControllerState(0, 0, 0, 0, 0, 0, 0);

    /** Every button bit that exists. */
    public static final int BUTTON_MASK = (1 << ControllerInput.Button.values().length) - 1;

    /** Clamp values coming off the network into range. */
    public ControllerState {
        buttons &= BUTTON_MASK;
        lx = clampAxis(lx);
        ly = clampAxis(ly);
        rx = clampAxis(rx);
        ry = clampAxis(ry);
        lt = Math.max(0, Math.min(255, lt));
        rt = Math.max(0, Math.min(255, rt));
    }

    private static int clampAxis(int v) {
        return Math.max(-127, Math.min(127, v));
    }

    /** The state with exactly {@code pressed} inputs held (keys are digital). */
    public static ControllerState of(Set<ControllerInput> pressed) {
        int buttons = 0;
        for (ControllerInput in : pressed) {
            if (in.button() != null) buttons |= in.button().bit();
        }
        return new ControllerState(buttons,
                axis(pressed, ControllerInput.LS_RIGHT, ControllerInput.LS_LEFT),
                axis(pressed, ControllerInput.LS_UP, ControllerInput.LS_DOWN),
                axis(pressed, ControllerInput.RS_RIGHT, ControllerInput.RS_LEFT),
                axis(pressed, ControllerInput.RS_UP, ControllerInput.RS_DOWN),
                pressed.contains(ControllerInput.LT) ? 255 : 0,
                pressed.contains(ControllerInput.RT) ? 255 : 0);
    }

    private static int axis(Set<ControllerInput> pressed, ControllerInput plus, ControllerInput minus) {
        return (pressed.contains(plus) ? 127 : 0) - (pressed.contains(minus) ? 127 : 0);
    }

    public boolean isDown(ControllerInput.Button b) {
        return (buttons & b.bit()) != 0;
    }

    /** Axis by name: lx, ly, rx, ry (-1..1) or lt, rt (0..1); NaN for an unknown name. */
    public double axis(String name) {
        return switch (name) {
            case "lx" -> lx / 127.0;
            case "ly" -> ly / 127.0;
            case "rx" -> rx / 127.0;
            case "ry" -> ry / 127.0;
            case "lt" -> lt / 255.0;
            case "rt" -> rt / 255.0;
            default -> Double.NaN;
        };
    }

    /** Axis names, in the order of {@link #rawAxes()}. */
    public static final String[] AXES = {"lx", "ly", "rx", "ry", "lt", "rt"};

    public int[] rawAxes() {
        return new int[]{lx, ly, rx, ry, lt, rt};
    }
}
