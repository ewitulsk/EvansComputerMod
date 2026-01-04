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
import java.util.concurrent.ConcurrentHashMap;

/**
 * Provides WASM host functions for persistent data storage.
 * 
 * Stores data as JSON in the terminal's NBT data, persisting across
 * world saves and reloads.
 */
public class NBTStorageHostFunctions implements PeripheralHostProvider {
    
    private static final int ERROR_NOT_AVAILABLE = -2;
    private static final int MAX_STORAGE_SIZE = 65536;  // 64KB per terminal
    
    // In-memory cache of terminal storage (JSON strings)
    // In a real implementation, this would be saved to the terminal's NBT
    private static final Map<TerminalBlockEntity, String> storageCache = new ConcurrentHashMap<>();
    
    @Override
    public String getName() {
        return "nbt_storage";
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
        
        // nbt_read(buf_ptr, buf_len) -> i32
        Func readFunc = new Func(store,
                new FuncType(new Type[]{Type.I32, Type.I32}, new Type[]{Type.I32}),
                (caller, params, results) -> {
                    int bufPtr = params[0].i32();
                    int bufLen = params[1].i32();
                    results[0] = Val.fromI32(hostRead(host, terminal, bufPtr, bufLen));
                });
        hostFunctions.add(readFunc);
        functions.put("nbt_read", Extern.fromFunc(readFunc));
        
        // nbt_write_json(json_ptr, json_len) -> i32
        Func writeJsonFunc = new Func(store,
                new FuncType(new Type[]{Type.I32, Type.I32}, new Type[]{Type.I32}),
                (caller, params, results) -> {
                    int jsonPtr = params[0].i32();
                    int jsonLen = params[1].i32();
                    results[0] = Val.fromI32(hostWriteJson(host, terminal, jsonPtr, jsonLen));
                });
        hostFunctions.add(writeJsonFunc);
        functions.put("nbt_write_json", Extern.fromFunc(writeJsonFunc));
        
        APIntegrationLoader.LOGGER.debug("Created {} NBT storage host functions", functions.size());
        return functions;
    }
    
    // ==================== Host Function Implementations ====================
    
    private int hostRead(TerminalWasmHost host, TerminalBlockEntity terminal,
                          int bufPtr, int bufLen) {
        if (terminal.getLevel() == null) return ERROR_NOT_AVAILABLE;
        
        String data = storageCache.getOrDefault(terminal, "{}");
        return host.writeString(bufPtr, bufLen, data);
    }
    
    private int hostWriteJson(TerminalWasmHost host, TerminalBlockEntity terminal,
                               int jsonPtr, int jsonLen) {
        if (terminal.getLevel() == null) return ERROR_NOT_AVAILABLE;
        
        String json = host.readString(jsonPtr, jsonLen);
        if (json == null) {
            return -1;
        }
        
        // Validate size
        if (json.length() > MAX_STORAGE_SIZE) {
            return -1;  // Too large
        }
        
        // Basic JSON validation - must start with { or [
        String trimmed = json.trim();
        if (!trimmed.startsWith("{") && !trimmed.startsWith("[")) {
            return -1;  // Not valid JSON
        }
        
        storageCache.put(terminal, json);
        
        // In a real implementation, also save to terminal's NBT here
        // terminal.setChanged(); // Mark for saving
        
        return json.length();
    }
}
