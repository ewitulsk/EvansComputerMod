package com.example.evanscomputermod.controller;

import com.example.evanscomputermod.api.peripheral.AnnotatedPeripheral;
import com.example.evanscomputermod.api.peripheral.IComputerAccess;
import com.example.evanscomputermod.api.peripheral.PeripheralException;
import com.example.evanscomputermod.api.peripheral.PeripheralMethod;

import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.concurrent.CopyOnWriteArraySet;

/**
 * A connected Wireless Xbox Controller, as a program sees it: peripheral type
 * {@code xbox_controller}, attached as {@code controller_1} .. {@code controller_4}
 * (the player number) while the controller is connected.
 *
 * <p>Programs only see buttons and axes, never keys: the key mapping lives on
 * the controller item. Reads don't wait for the server thread.
 *
 * <p>Events, as {@code (event, attachment, ...)}:
 * <ul>
 *   <li>{@code ("controller_button", name, button, pressed)}</li>
 *   <li>{@code ("controller_axis", name, axis, value)}: lx, ly, rx, ry in -1..1, lt, rt in 0..1</li>
 * </ul>
 * plus the usual {@code peripheral} / {@code peripheral_detach} when it
 * connects and disconnects.
 */
public final class ControllerPeripheral extends AnnotatedPeripheral {

    public static final String TYPE = "xbox_controller";

    private final int player;
    private final Set<IComputerAccess> computers = new CopyOnWriteArraySet<>();
    private volatile ControllerState state = ControllerState.NEUTRAL;

    public ControllerPeripheral(int player) {
        this.player = player;
    }

    @Override
    public String getType() {
        return TYPE;
    }

    @Override
    public void attach(IComputerAccess computer) {
        computers.add(computer);
    }

    @Override
    public void detach(IComputerAccess computer) {
        computers.remove(computer);
    }

    public ControllerState state() {
        return state;
    }

    /** New input from the player. Posts events for whatever changed. */
    public void update(ControllerState next) {
        ControllerState prev = state;
        state = next;
        if (prev.equals(next)) return;
        int changed = prev.buttons() ^ next.buttons();
        for (ControllerInput.Button b : ControllerInput.Button.values()) {
            if ((changed & b.bit()) != 0) event("controller_button", b.id(), next.isDown(b));
        }
        int[] a = prev.rawAxes();
        int[] n = next.rawAxes();
        for (int i = 0; i < a.length; i++) {
            if (a[i] != n[i]) event("controller_axis", ControllerState.AXES[i], next.axis(ControllerState.AXES[i]));
        }
    }

    private void event(String name, Object... args) {
        for (IComputerAccess c : computers) c.queueEvent(name, args);
    }

    // ------------------------------------------------------------ methods

    @PeripheralMethod(mainThread = false,
            description = "Everything at once: {a: bool, ..., lx, ly, rx, ry (-1..1), lt, rt (0..1), player}")
    public Map<String, Object> getState() {
        ControllerState s = state;
        Map<String, Object> out = new LinkedHashMap<>();
        for (ControllerInput.Button b : ControllerInput.Button.values()) out.put(b.id(), s.isDown(b));
        for (String axis : ControllerState.AXES) out.put(axis, s.axis(axis));
        out.put("player", player);
        return out;
    }

    @PeripheralMethod(mainThread = false, description = "Whether a button is held: a, b, x, y, lb, rb, back, start, guide, ls, rs, dpad_up, ...")
    public boolean isDown(String button) throws PeripheralException {
        ControllerInput.Button b = ControllerInput.Button.byId(button);
        if (b == null) throw new PeripheralException("unknown button '" + button + "'");
        return state.isDown(b);
    }

    @PeripheralMethod(mainThread = false, description = "Names of the buttons held now")
    public List<String> getButtons() {
        ControllerState s = state;
        List<String> out = new ArrayList<>();
        for (ControllerInput.Button b : ControllerInput.Button.values()) {
            if (s.isDown(b)) out.add(b.id());
        }
        return out;
    }

    @PeripheralMethod(mainThread = false, description = "One axis: lx, ly, rx, ry (-1..1, up and right positive) or lt, rt (0..1)")
    public double getAxis(String axis) throws PeripheralException {
        double v = state.axis(axis);
        if (Double.isNaN(v)) throw new PeripheralException("unknown axis '" + axis + "'");
        return v;
    }

    @PeripheralMethod(mainThread = false,
            description = "Raw state for drivers: [buttons bit mask, lx, ly, rx, ry (-127..127), lt, rt (0..255)]")
    public List<Integer> getRaw() {
        ControllerState s = state;
        return List.of(s.buttons(), s.lx(), s.ly(), s.rx(), s.ry(), s.lt(), s.rt());
    }

    @PeripheralMethod(mainThread = false, description = "Player number, 1-4")
    public int getPlayer() {
        return player;
    }

    @PeripheralMethod(mainThread = false, description = "Button names in bit order (bit 0 first), for get_raw")
    public List<String> getButtonNames() {
        List<String> out = new ArrayList<>();
        for (ControllerInput.Button b : ControllerInput.Button.values()) out.add(b.id());
        return out;
    }
}
