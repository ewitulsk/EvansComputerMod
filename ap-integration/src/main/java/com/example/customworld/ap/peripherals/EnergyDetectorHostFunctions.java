package com.example.customworld.ap.peripherals;

import com.example.customworld.ap.APIntegrationLoader;
import com.example.customworld.block.TerminalBlockEntity;
import com.example.customworld.wasm.PeripheralHostProvider;
import com.example.customworld.wasm.TerminalWasmHost;
import io.github.kawamuray.wasmtime.Extern;
import io.github.kawamuray.wasmtime.Func;
import io.github.kawamuray.wasmtime.FuncType;
import io.github.kawamuray.wasmtime.Store;
import io.github.kawamuray.wasmtime.Val;
import io.github.kawamuray.wasmtime.Val.Type;

import java.util.HashMap;
import java.util.List;
import java.util.Map;
import java.util.concurrent.atomic.AtomicInteger;

/**
 * Provides WASM host functions for energy monitoring.
 * 
 * Note: Since the terminal is not an actual energy detector block,
 * this provides simulated energy monitoring functionality that could
 * be extended to work with adjacent energy-carrying blocks.
 */
public class EnergyDetectorHostFunctions implements PeripheralHostProvider {
    
    private static final int ERROR_NOT_AVAILABLE = -2;
    
    // Per-terminal simulated energy state
    private static final Map<TerminalBlockEntity, EnergyState> energyStates = new HashMap<>();
    
    private static class EnergyState {
        AtomicInteger transferRate = new AtomicInteger(0);
        AtomicInteger transferRateLimit = new AtomicInteger(Integer.MAX_VALUE);
    }
    
    private static EnergyState getState(TerminalBlockEntity terminal) {
        return energyStates.computeIfAbsent(terminal, k -> new EnergyState());
    }
    
    @Override
    public String getName() {
        return "energy_detector";
    }
    
    @Override
    public boolean isAvailable() {
        try {
            Class.forName("de.srendi.advancedperipherals.AdvancedPeripherals");
            return true;
        } catch (ClassNotFoundException e) {
            return false;
        }
    }
    
    @Override
    public Map<String, Extern> createHostFunctions(TerminalWasmHost host, List<Func> hostFunctions) {
        Map<String, Extern> functions = new HashMap<>();
        Store<Void> store = host.getStore();
        TerminalBlockEntity terminal = host.getTerminal();
        
        // energy_get_transfer_rate() -> i32
        Func getTransferRateFunc = new Func(store,
                new FuncType(new Type[]{}, new Type[]{Type.I32}),
                (caller, params, results) -> {
                    results[0] = Val.fromI32(hostGetTransferRate(terminal));
                });
        hostFunctions.add(getTransferRateFunc);
        functions.put("energy_get_transfer_rate", Extern.fromFunc(getTransferRateFunc));
        
        // energy_get_transfer_rate_limit() -> i32
        Func getTransferRateLimitFunc = new Func(store,
                new FuncType(new Type[]{}, new Type[]{Type.I32}),
                (caller, params, results) -> {
                    results[0] = Val.fromI32(hostGetTransferRateLimit(terminal));
                });
        hostFunctions.add(getTransferRateLimitFunc);
        functions.put("energy_get_transfer_rate_limit", Extern.fromFunc(getTransferRateLimitFunc));
        
        // energy_set_transfer_rate_limit(rate) -> i32
        Func setTransferRateLimitFunc = new Func(store,
                new FuncType(new Type[]{Type.I32}, new Type[]{Type.I32}),
                (caller, params, results) -> {
                    int rate = params[0].i32();
                    results[0] = Val.fromI32(hostSetTransferRateLimit(terminal, rate));
                });
        hostFunctions.add(setTransferRateLimitFunc);
        functions.put("energy_set_transfer_rate_limit", Extern.fromFunc(setTransferRateLimitFunc));
        
        APIntegrationLoader.LOGGER.debug("Created {} energy detector host functions", functions.size());
        return functions;
    }
    
    // ==================== Host Function Implementations ====================
    
    private int hostGetTransferRate(TerminalBlockEntity terminal) {
        if (terminal.getLevel() == null) return ERROR_NOT_AVAILABLE;
        
        // In a real implementation, this would read from adjacent energy blocks
        // For now, return a simulated value
        EnergyState state = getState(terminal);
        return state.transferRate.get();
    }
    
    private int hostGetTransferRateLimit(TerminalBlockEntity terminal) {
        if (terminal.getLevel() == null) return ERROR_NOT_AVAILABLE;
        
        EnergyState state = getState(terminal);
        int limit = state.transferRateLimit.get();
        return limit == Integer.MAX_VALUE ? -1 : limit;  // -1 means unlimited
    }
    
    private int hostSetTransferRateLimit(TerminalBlockEntity terminal, int rate) {
        if (terminal.getLevel() == null) return ERROR_NOT_AVAILABLE;
        
        EnergyState state = getState(terminal);
        state.transferRateLimit.set(Math.max(0, rate));
        return 0;  // Success
    }
}
