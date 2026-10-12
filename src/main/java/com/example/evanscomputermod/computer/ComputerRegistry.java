package com.example.evanscomputermod.computer;

import com.example.evanscomputermod.api.IComputerHost;

import org.jetbrains.annotations.Nullable;
import java.util.UUID;
import java.util.concurrent.ConcurrentHashMap;

/**
 * Global registry of loaded computer hosts, keyed by UUID (Terminals register
 * when they load, booted or not).
 * Used for UUID-based computer lookup (e.g., from network packets targeting
 * computers that may not be block entities).
 */
public class ComputerRegistry {

    private static final ConcurrentHashMap<UUID, IComputerHost> activeComputers = new ConcurrentHashMap<>();

    public static void register(IComputerHost host) {
        activeComputers.put(host.getComputerId(), host);
    }

    public static void unregister(UUID computerId) {
        activeComputers.remove(computerId);
    }

    /** Unregister only if {@code host} is still the one registered (a moved block's successor may have taken over). */
    public static void unregister(UUID computerId, IComputerHost host) {
        activeComputers.remove(computerId, host);
    }

    @Nullable
    public static IComputerHost get(UUID computerId) {
        return activeComputers.get(computerId);
    }

    public static int getActiveCount() {
        return activeComputers.size();
    }
}
