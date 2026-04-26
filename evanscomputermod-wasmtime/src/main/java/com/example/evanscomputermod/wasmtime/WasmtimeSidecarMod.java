package com.example.evanscomputermod.wasmtime;

import net.neoforged.bus.api.IEventBus;
import net.neoforged.fml.common.Mod;
import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

/**
 * @Mod entry point for the Wasmtime sidecar. Its only job is to be on the
 * NeoForge mod classpath so {@link WasmtimeRuntimeProvider} is discovered
 * by ServiceLoader from the main mod at server start. No other state.
 */
@Mod("evanscomputermod_wasmtime")
public class WasmtimeSidecarMod {
    private static final Logger LOGGER = LoggerFactory.getLogger(WasmtimeSidecarMod.class);

    public WasmtimeSidecarMod(IEventBus modEventBus) {
        LOGGER.info(
                "Wasmtime runtime sidecar loaded — main mod will prefer it over Chicory "
              + "if its native libraries can be loaded on this platform.");
    }
}
