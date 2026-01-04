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
import net.minecraft.core.BlockPos;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.resources.ResourceLocation;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.block.state.BlockState;
import net.neoforged.neoforge.common.Tags;

import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Map;

/**
 * Provides WASM host functions for block scanning.
 * 
 * Mirrors Advanced Peripherals' GeoScannerPeripheral functionality.
 */
public class GeoScannerHostFunctions implements PeripheralHostProvider {
    
    private static final int ERROR_NOT_AVAILABLE = -2;
    private static final int MAX_RADIUS = 16;  // Limit for performance
    
    @Override
    public String getName() {
        return "geo_scanner";
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
        
        // geo_scan(radius, buf_ptr, buf_len) -> i32
        Func scanFunc = new Func(store,
                new FuncType(new Type[]{Type.I32, Type.I32, Type.I32}, new Type[]{Type.I32}),
                (caller, params, results) -> {
                    int radius = params[0].i32();
                    int bufPtr = params[1].i32();
                    int bufLen = params[2].i32();
                    results[0] = Val.fromI32(hostScan(host, terminal, radius, bufPtr, bufLen));
                });
        hostFunctions.add(scanFunc);
        functions.put("geo_scan", Extern.fromFunc(scanFunc));
        
        // geo_chunk_analyze(buf_ptr, buf_len) -> i32
        Func chunkAnalyzeFunc = new Func(store,
                new FuncType(new Type[]{Type.I32, Type.I32}, new Type[]{Type.I32}),
                (caller, params, results) -> {
                    int bufPtr = params[0].i32();
                    int bufLen = params[1].i32();
                    results[0] = Val.fromI32(hostChunkAnalyze(host, terminal, bufPtr, bufLen));
                });
        hostFunctions.add(chunkAnalyzeFunc);
        functions.put("geo_chunk_analyze", Extern.fromFunc(chunkAnalyzeFunc));
        
        // geo_scan_cost(radius) -> i32
        Func scanCostFunc = new Func(store,
                new FuncType(new Type[]{Type.I32}, new Type[]{Type.I32}),
                (caller, params, results) -> {
                    int radius = params[0].i32();
                    results[0] = Val.fromI32(hostScanCost(radius));
                });
        hostFunctions.add(scanCostFunc);
        functions.put("geo_scan_cost", Extern.fromFunc(scanCostFunc));
        
        APIntegrationLoader.LOGGER.debug("Created {} geo scanner host functions", functions.size());
        return functions;
    }
    
    // ==================== Host Function Implementations ====================
    
    private int hostScan(TerminalWasmHost host, TerminalBlockEntity terminal,
                          int radius, int bufPtr, int bufLen) {
        if (terminal.getLevel() == null) return ERROR_NOT_AVAILABLE;
        
        radius = Math.min(Math.max(1, radius), MAX_RADIUS);
        Level level = terminal.getLevel();
        BlockPos center = terminal.getBlockPos();
        
        List<Map<String, Object>> blocks = new ArrayList<>();
        
        for (int x = -radius; x <= radius; x++) {
            for (int y = -radius; y <= radius; y++) {
                for (int z = -radius; z <= radius; z++) {
                    BlockPos pos = center.offset(x, y, z);
                    BlockState state = level.getBlockState(pos);
                    
                    if (state.isAir()) continue;
                    
                    ResourceLocation name = BuiltInRegistries.BLOCK.getKey(state.getBlock());
                    if (name == null) continue;
                    
                    Map<String, Object> data = new HashMap<>();
                    data.put("x", x);
                    data.put("y", y);
                    data.put("z", z);
                    data.put("name", name.toString());
                    blocks.add(data);
                }
            }
        }
        
        String json = toJsonArray(blocks);
        return host.writeString(bufPtr, bufLen, json);
    }
    
    private int hostChunkAnalyze(TerminalWasmHost host, TerminalBlockEntity terminal,
                                  int bufPtr, int bufLen) {
        if (terminal.getLevel() == null) return ERROR_NOT_AVAILABLE;
        
        Level level = terminal.getLevel();
        BlockPos termPos = terminal.getBlockPos();
        int chunkX = termPos.getX() >> 4;
        int chunkZ = termPos.getZ() >> 4;
        
        Map<String, Integer> oreCount = new HashMap<>();
        
        for (int x = chunkX * 16; x < (chunkX + 1) * 16; x++) {
            for (int z = chunkZ * 16; z < (chunkZ + 1) * 16; z++) {
                for (int y = level.getMinBuildHeight(); y < level.getHeight(); y++) {
                    BlockPos pos = new BlockPos(x, y, z);
                    BlockState state = level.getBlockState(pos);
                    
                    if (state.is(Tags.Blocks.ORES)) {
                        ResourceLocation name = BuiltInRegistries.BLOCK.getKey(state.getBlock());
                        if (name != null) {
                            oreCount.merge(name.toString(), 1, Integer::sum);
                        }
                    }
                }
            }
        }
        
        String json = toJsonObject(oreCount);
        return host.writeString(bufPtr, bufLen, json);
    }
    
    private int hostScanCost(int radius) {
        if (radius < 1) return 0;
        if (radius > MAX_RADIUS) return -1;
        // Simple cost calculation based on volume
        int volume = (2 * radius + 1) * (2 * radius + 1) * (2 * radius + 1);
        return volume / 100;  // 1 fuel per 100 blocks scanned
    }
    
    private String toJsonArray(List<Map<String, Object>> list) {
        StringBuilder sb = new StringBuilder("[");
        for (int i = 0; i < list.size(); i++) {
            if (i > 0) sb.append(",");
            sb.append(toJsonObject(list.get(i)));
        }
        sb.append("]");
        return sb.toString();
    }
    
    private String toJsonObject(Map<?, ?> map) {
        StringBuilder sb = new StringBuilder("{");
        boolean first = true;
        for (Map.Entry<?, ?> entry : map.entrySet()) {
            if (!first) sb.append(",");
            sb.append("\"").append(entry.getKey()).append("\":");
            Object val = entry.getValue();
            if (val instanceof String) {
                sb.append("\"").append(escapeJson((String) val)).append("\"");
            } else if (val instanceof Number) {
                sb.append(val);
            } else {
                sb.append("\"").append(val).append("\"");
            }
            first = false;
        }
        sb.append("}");
        return sb.toString();
    }
    
    private String escapeJson(String s) {
        return s.replace("\\", "\\\\").replace("\"", "\\\"");
    }
}
