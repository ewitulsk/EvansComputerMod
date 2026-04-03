package com.example.evanscomputermod.computer;

import com.example.evanscomputermod.api.IComputerHost;

import org.jspecify.annotations.Nullable;
import java.util.UUID;
import java.util.concurrent.ConcurrentHashMap;

/**
 * Global registry of active computer instances, keyed by UUID.
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

    @Nullable
    public static IComputerHost get(UUID computerId) {
        return activeComputers.get(computerId);
    }

    public static int getActiveCount() {
        return activeComputers.size();
    }
}
