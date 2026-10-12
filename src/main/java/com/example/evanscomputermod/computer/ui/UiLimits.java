package com.example.evanscomputermod.computer.ui;

import com.example.evanscomputermod.EcmConfig;

/**
 * The per-app limits of §6, server-configurable in the {@code [ui]} section
 * of the common config ({@link EcmConfig}). Tests build their own.
 */
public record UiLimits(int nodesPerSurface, int nodesPerApp, long resourceBytesPerApp, long treeOpsBytesPerSecond,
                       int surfacesPerApp, int connections, int shellTimeoutMs, long maxQueuedBytes) {

    public static UiLimits fromConfig() {
        return new UiLimits(EcmConfig.uiNodesPerSurface(), EcmConfig.uiNodesPerApp(), EcmConfig.uiResourceBytesPerApp(),
                EcmConfig.uiTreeOpsBytesPerSecond(), EcmConfig.uiSurfacesPerApp(), EcmConfig.uiConnections(),
                EcmConfig.uiShellTimeoutMs(), 8L << 20);
    }

    /** The protocol's defaults (§6). */
    public static UiLimits defaults() {
        return new UiLimits(8192, 32768, 8L << 20, 256L << 10, 64, 32, 5000, 8L << 20);
    }

    public UiLimits withNodesPerSurface(int n) {
        return new UiLimits(n, nodesPerApp, resourceBytesPerApp, treeOpsBytesPerSecond, surfacesPerApp, connections,
                shellTimeoutMs, maxQueuedBytes);
    }

    public UiLimits withResourceBytes(long n) {
        return new UiLimits(nodesPerSurface, nodesPerApp, n, treeOpsBytesPerSecond, surfacesPerApp, connections,
                shellTimeoutMs, maxQueuedBytes);
    }

    public UiLimits withTreeOpsRate(long n) {
        return new UiLimits(nodesPerSurface, nodesPerApp, resourceBytesPerApp, n, surfacesPerApp, connections,
                shellTimeoutMs, maxQueuedBytes);
    }
}
