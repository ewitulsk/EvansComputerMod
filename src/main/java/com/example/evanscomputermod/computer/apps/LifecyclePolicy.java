package com.example.evanscomputermod.computer.apps;

/**
 * How a device treats apps it isn't showing (UI_PROTOCOL.md §11; decided).
 */
public enum LifecyclePolicy {
    /**
     * Terminals: apps keep running; minimized windows just stop getting
     * {@code Frame} events. State is saved at shutdown and reboot and handed
     * back as {@code Restore} on the next launch.
     */
    PC,
    /**
     * ePhones: apps off screen are suspended (paused in memory) after
     * {@code WillSuspend}/{@code SaveState}; refresh wakes on a budget and
     * permission-gated exception modes run through {@link BackgroundScheduler}.
     */
    PHONE
}
