package com.example.evanscomputermod.missile;

import java.util.UUID;
import java.util.concurrent.ConcurrentHashMap;

/**
 * Thread-safe store for in-flight missile telemetry snapshots.
 *
 * Updated every tick by {@code MissileEntity} on the server thread.
 * Read any time by the WASM bridge thread via {@code MissileComputerModule}.
 * No extra network traffic — Python polls the store directly.
 */
public final class MissileTelemetryStore {

    private static final ConcurrentHashMap<UUID, TelemetrySnapshot> STORE = new ConcurrentHashMap<>();

    private MissileTelemetryStore() {}

    public static void put(UUID launchId, TelemetrySnapshot snapshot) {
        STORE.put(launchId, snapshot);
    }

    public static TelemetrySnapshot get(UUID launchId) {
        return STORE.get(launchId);
    }

    public static void remove(UUID launchId) {
        STORE.remove(launchId);
    }

    public static boolean contains(UUID launchId) {
        return STORE.containsKey(launchId);
    }
}
