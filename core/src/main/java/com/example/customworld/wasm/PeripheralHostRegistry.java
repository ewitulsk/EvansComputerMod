package com.example.customworld.wasm;

import com.example.customworld.CustomWorldMod;
import io.github.kawamuray.wasmtime.Extern;
import io.github.kawamuray.wasmtime.Func;

import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import java.util.concurrent.CopyOnWriteArrayList;

/**
 * Registry for peripheral host function providers.
 * 
 * This allows optional integration modules (like the Advanced Peripherals integration)
 * to register their host function providers without requiring them as dependencies.
 * 
 * Providers are registered during mod initialization and are queried when creating
 * a new WASM host for a terminal.
 */
public final class PeripheralHostRegistry {
    
    private static final List<PeripheralHostProvider> providers = new CopyOnWriteArrayList<>();
    
    private PeripheralHostRegistry() {
        // Utility class, no instantiation
    }
    
    /**
     * Registers a peripheral host function provider.
     * 
     * This should be called during mod initialization by integration modules.
     * 
     * @param provider The provider to register
     */
    public static void register(PeripheralHostProvider provider) {
        if (provider == null) {
            throw new IllegalArgumentException("Provider cannot be null");
        }
        
        providers.add(provider);
        CustomWorldMod.LOGGER.info("Registered peripheral host provider: {}", provider.getName());
    }
    
    /**
     * Unregisters a peripheral host function provider.
     * 
     * @param provider The provider to unregister
     * @return true if the provider was found and removed
     */
    public static boolean unregister(PeripheralHostProvider provider) {
        boolean removed = providers.remove(provider);
        if (removed) {
            CustomWorldMod.LOGGER.info("Unregistered peripheral host provider: {}", provider.getName());
        }
        return removed;
    }
    
    /**
     * Creates all host functions from registered providers.
     * 
     * This method is called by {@link TerminalWasmHost} when creating host functions.
     * It iterates through all registered providers and collects their host functions.
     * 
     * @param host The terminal WASM host instance
     * @param hostFunctions A list to add created Func instances to (for lifecycle management)
     * @return A map of all host function names to their Extern instances
     */
    public static Map<String, Extern> createAllHostFunctions(
            TerminalWasmHost host,
            List<Func> hostFunctions) {
        
        Map<String, Extern> allFunctions = new HashMap<>();
        
        for (PeripheralHostProvider provider : providers) {
            if (!provider.isAvailable()) {
                CustomWorldMod.LOGGER.debug("Skipping unavailable provider: {}", provider.getName());
                continue;
            }
            
            try {
                Map<String, Extern> providerFunctions = provider.createHostFunctions(host, hostFunctions);
                
                // Check for name collisions
                for (String name : providerFunctions.keySet()) {
                    if (allFunctions.containsKey(name)) {
                        CustomWorldMod.LOGGER.warn(
                            "Host function name collision: '{}' from provider '{}' already exists",
                            name, provider.getName()
                        );
                    }
                }
                
                allFunctions.putAll(providerFunctions);
                CustomWorldMod.LOGGER.debug(
                    "Added {} host functions from provider: {}", 
                    providerFunctions.size(), provider.getName()
                );
                
            } catch (Exception e) {
                CustomWorldMod.LOGGER.error(
                    "Failed to create host functions from provider: {}", 
                    provider.getName(), e
                );
            }
        }
        
        return allFunctions;
    }
    
    /**
     * Returns the number of registered providers.
     * 
     * @return The provider count
     */
    public static int getProviderCount() {
        return providers.size();
    }
    
    /**
     * Returns a list of registered provider names.
     * 
     * @return List of provider names
     */
    public static List<String> getProviderNames() {
        List<String> names = new ArrayList<>();
        for (PeripheralHostProvider provider : providers) {
            names.add(provider.getName());
        }
        return names;
    }
}
