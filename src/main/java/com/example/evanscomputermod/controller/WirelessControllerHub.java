package com.example.evanscomputermod.controller;

import com.example.evanscomputermod.computer.peripheral.PeripheralHub;

import java.util.UUID;

/**
 * The wireless receiver of one computer: up to four controllers connected at
 * once, attached to the computer's peripherals as {@code controller_1} ..
 * {@code controller_4} in the order they connected (the player numbers).
 * A controller that stops sending (out of range, the player left, the game
 * crashed) is dropped after {@link #TIMEOUT_MS}, so no button stays held.
 *
 * <p>Server thread only.
 */
public final class WirelessControllerHub {

    public static final int MAX_CONTROLLERS = 4;
    /** A connected controller sends at least every 0.5 s; silence this long disconnects it. */
    public static final long TIMEOUT_MS = 2_000;

    private static final class Slot {
        final UUID controllerId;
        final UUID playerId;
        final ControllerPeripheral peripheral;
        long lastSeenMs;

        Slot(UUID controllerId, UUID playerId, ControllerPeripheral peripheral, long now) {
            this.controllerId = controllerId;
            this.playerId = playerId;
            this.peripheral = peripheral;
            this.lastSeenMs = now;
        }
    }

    private final PeripheralHub hub;
    private final Slot[] slots = new Slot[MAX_CONTROLLERS];

    public WirelessControllerHub(PeripheralHub hub) {
        this.hub = hub;
    }

    public static String attachmentName(int player) {
        return "controller_" + player;
    }

    /**
     * Input from a connected controller. Connects it if it isn't yet.
     *
     * @return its player number (1-4), or 0 if all four slots are taken
     */
    public int update(UUID controllerId, UUID playerId, ControllerState state, long now) {
        int i = find(controllerId);
        if (i < 0) {
            i = freeSlot();
            if (i < 0) return 0;
            Slot s = new Slot(controllerId, playerId, new ControllerPeripheral(i + 1), now);
            slots[i] = s;
            hub.setWireless(attachmentName(i + 1), s.peripheral);
        }
        Slot s = slots[i];
        s.lastSeenMs = now;
        s.peripheral.update(state);
        return i + 1;
    }

    /** Player number of a connected controller, or 0. */
    public int playerOf(UUID controllerId) {
        int i = find(controllerId);
        return i < 0 ? 0 : i + 1;
    }

    /** Disconnect a controller (releasing everything it held). Returns true if it was connected. */
    public boolean disconnect(UUID controllerId) {
        int i = find(controllerId);
        if (i < 0) return false;
        drop(i);
        return true;
    }

    /** Drop controllers that went silent. */
    public void tick(long now) {
        for (int i = 0; i < slots.length; i++) {
            if (slots[i] != null && now - slots[i].lastSeenMs > TIMEOUT_MS) drop(i);
        }
    }

    /** Disconnect everything (computer removed or unloaded). */
    public void disconnectAll() {
        for (int i = 0; i < slots.length; i++) {
            if (slots[i] != null) drop(i);
        }
    }

    public int connectedCount() {
        int n = 0;
        for (Slot s : slots) if (s != null) n++;
        return n;
    }

    private void drop(int i) {
        Slot s = slots[i];
        slots[i] = null;
        // Programs reading the peripheral directly see everything released first.
        s.peripheral.update(ControllerState.NEUTRAL);
        hub.setWireless(attachmentName(i + 1), null);
    }

    private int find(UUID controllerId) {
        for (int i = 0; i < slots.length; i++) {
            if (slots[i] != null && slots[i].controllerId.equals(controllerId)) return i;
        }
        return -1;
    }

    private int freeSlot() {
        for (int i = 0; i < slots.length; i++) {
            if (slots[i] == null) return i;
        }
        return -1;
    }
}
