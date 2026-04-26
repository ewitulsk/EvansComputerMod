package com.example.evanscomputermod.api.wasm;

import com.example.evanscomputermod.EvansComputerMod;

import java.util.ArrayList;
import java.util.Comparator;
import java.util.List;
import java.util.ServiceLoader;

/**
 * Loads {@link WasmRuntimeProvider} instances via {@link ServiceLoader},
 * filters and ranks them, and produces the chosen {@link WasmRuntime}.
 */
public final class WasmRuntimeRegistry {

    private WasmRuntimeRegistry() {}

    private static volatile WasmRuntime selected;
    private static volatile String selectedProviderName;

    /**
     * Discover providers and choose the highest-priority available one whose
     * {@code apiVersion} matches the current SPI. Idempotent — subsequent
     * calls return the cached runtime.
     *
     * @throws IllegalStateException if no compatible provider is available.
     */
    public static synchronized WasmRuntime select() {
        if (selected != null) return selected;

        List<WasmRuntimeProvider> all = new ArrayList<>();
        for (WasmRuntimeProvider p : ServiceLoader.load(
                WasmRuntimeProvider.class, WasmRuntimeRegistry.class.getClassLoader())) {
            all.add(p);
        }

        if (all.isEmpty()) {
            throw new IllegalStateException(
                    "No WasmRuntimeProvider found via ServiceLoader. The main mod is supposed to "
                  + "ship a Chicory provider — check that META-INF/services is in the jar.");
        }

        List<WasmRuntimeProvider> compatible = new ArrayList<>();
        for (WasmRuntimeProvider p : all) {
            if (p.apiVersion() != WasmRuntimeProvider.CURRENT_API_VERSION) {
                EvansComputerMod.LOGGER.warn(
                        "WASM runtime provider '{}' rejected: apiVersion {} != current {}",
                        p.name(), p.apiVersion(), WasmRuntimeProvider.CURRENT_API_VERSION);
                continue;
            }
            if (!p.isAvailable()) {
                EvansComputerMod.LOGGER.info(
                        "WASM runtime provider '{}' reports unavailable (priority {})",
                        p.name(), p.priority());
                continue;
            }
            compatible.add(p);
        }

        if (compatible.isEmpty()) {
            StringBuilder sb = new StringBuilder("No usable WasmRuntimeProvider. Discovered: ");
            for (WasmRuntimeProvider p : all) {
                sb.append("[name=").append(p.name())
                  .append(", priority=").append(p.priority())
                  .append(", apiVersion=").append(p.apiVersion())
                  .append(", available=").append(safeIsAvailable(p))
                  .append("] ");
            }
            throw new IllegalStateException(sb.toString());
        }

        compatible.sort(Comparator.comparingInt(WasmRuntimeProvider::priority).reversed());
        WasmRuntimeProvider winner = compatible.get(0);

        EvansComputerMod.LOGGER.info(
                "WASM runtime selected: '{}' (priority {}, apiVersion {}). Discovered {} provider(s).",
                winner.name(), winner.priority(), winner.apiVersion(), all.size());
        if (compatible.size() > 1) {
            for (int i = 1; i < compatible.size(); i++) {
                WasmRuntimeProvider p = compatible.get(i);
                EvansComputerMod.LOGGER.info("  also available: '{}' (priority {})", p.name(), p.priority());
            }
        }

        selected = winner.create();
        selectedProviderName = winner.name();
        return selected;
    }

    /** Returns the previously selected runtime, or {@code null} if not yet selected. */
    public static WasmRuntime runtime() {
        return selected;
    }

    /** Name of the currently selected provider, for log messages. */
    public static String selectedProviderName() {
        return selectedProviderName;
    }

    private static boolean safeIsAvailable(WasmRuntimeProvider p) {
        try {
            return p.isAvailable();
        } catch (Throwable t) {
            return false;
        }
    }
}
