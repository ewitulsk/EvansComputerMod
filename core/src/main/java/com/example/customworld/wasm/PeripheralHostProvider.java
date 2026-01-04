package com.example.customworld.wasm;

import io.github.kawamuray.wasmtime.Extern;
import io.github.kawamuray.wasmtime.Func;

import java.util.List;
import java.util.Map;

/**
 * Interface for peripheral providers that expose WASM host functions.
 * 
 * Implementations of this interface can be registered with {@link PeripheralHostRegistry}
 * to extend the WASM terminal with additional host functions.
 * 
 * This is primarily used by optional integration modules (like Advanced Peripherals)
 * to add peripheral functionality without requiring the integration as a dependency.
 */
public interface PeripheralHostProvider {
    
    /**
     * Returns the name of this peripheral provider.
     * Used for logging and debugging.
     * 
     * @return The provider name (e.g., "player_detector", "chat_box")
     */
    String getName();
    
    /**
     * Creates the host functions provided by this peripheral.
     * 
     * The returned map should contain function names as keys and their corresponding
     * {@link Extern} instances (wrapping {@link Func}) as values.
     * 
     * Function naming convention: {@code peripheral_name_function_name}
     * Example: {@code player_detector_get_online_players}
     * 
     * The host provides access to:
     * - {@link TerminalWasmHost#getStore()} - the Wasmtime store for creating functions
     * - {@link TerminalWasmHost#getTerminal()} - the terminal for context (position, level)
     * - {@link TerminalWasmHost#readString(int, int)} - read strings from WASM memory
     * - {@link TerminalWasmHost#writeString(int, int, String)} - write strings to WASM memory
     * 
     * @param host The terminal WASM host instance
     * @param hostFunctions A list to add created Func instances to (for lifecycle management)
     * @return A map of function names to Extern instances
     */
    Map<String, Extern> createHostFunctions(TerminalWasmHost host, List<Func> hostFunctions);
    
    /**
     * Checks if this peripheral provider is available.
     * 
     * This allows providers to check for optional dependencies at runtime.
     * For example, an Advanced Peripherals integration would return false
     * if AP is not installed.
     * 
     * @return true if the provider is available and can create host functions
     */
    default boolean isAvailable() {
        return true;
    }
}
