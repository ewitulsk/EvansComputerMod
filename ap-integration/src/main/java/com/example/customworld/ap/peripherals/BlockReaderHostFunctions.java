package com.example.customworld.ap.peripherals;

import com.example.customworld.ap.APIntegrationLoader;
import com.example.customworld.block.TerminalBlock;
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
import net.minecraft.core.Direction;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.nbt.CompoundTag;
import net.minecraft.resources.ResourceLocation;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.entity.BlockEntity;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.block.state.properties.Property;

import java.util.HashMap;
import java.util.List;
import java.util.Map;

/**
 * Provides WASM host functions for reading block information.
 * 
 * Reads the block in front of the terminal (based on facing direction).
 */
public class BlockReaderHostFunctions implements PeripheralHostProvider {
    
    private static final int ERROR_NOT_AVAILABLE = -2;
    
    @Override
    public String getName() {
        return "block_reader";
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
        
        // block_reader_get_name(buf_ptr, buf_len) -> i32
        Func getNameFunc = new Func(store,
                new FuncType(new Type[]{Type.I32, Type.I32}, new Type[]{Type.I32}),
                (caller, params, results) -> {
                    int bufPtr = params[0].i32();
                    int bufLen = params[1].i32();
                    results[0] = Val.fromI32(hostGetBlockName(host, terminal, bufPtr, bufLen));
                });
        hostFunctions.add(getNameFunc);
        functions.put("block_reader_get_name", Extern.fromFunc(getNameFunc));
        
        // block_reader_get_data(buf_ptr, buf_len) -> i32
        Func getDataFunc = new Func(store,
                new FuncType(new Type[]{Type.I32, Type.I32}, new Type[]{Type.I32}),
                (caller, params, results) -> {
                    int bufPtr = params[0].i32();
                    int bufLen = params[1].i32();
                    results[0] = Val.fromI32(hostGetBlockData(host, terminal, bufPtr, bufLen));
                });
        hostFunctions.add(getDataFunc);
        functions.put("block_reader_get_data", Extern.fromFunc(getDataFunc));
        
        // block_reader_get_states(buf_ptr, buf_len) -> i32
        Func getStatesFunc = new Func(store,
                new FuncType(new Type[]{Type.I32, Type.I32}, new Type[]{Type.I32}),
                (caller, params, results) -> {
                    int bufPtr = params[0].i32();
                    int bufLen = params[1].i32();
                    results[0] = Val.fromI32(hostGetBlockStates(host, terminal, bufPtr, bufLen));
                });
        hostFunctions.add(getStatesFunc);
        functions.put("block_reader_get_states", Extern.fromFunc(getStatesFunc));
        
        // block_reader_is_tile_entity() -> i32
        Func isTileEntityFunc = new Func(store,
                new FuncType(new Type[]{}, new Type[]{Type.I32}),
                (caller, params, results) -> {
                    results[0] = Val.fromI32(hostIsTileEntity(terminal));
                });
        hostFunctions.add(isTileEntityFunc);
        functions.put("block_reader_is_tile_entity", Extern.fromFunc(isTileEntityFunc));
        
        APIntegrationLoader.LOGGER.debug("Created {} block reader host functions", functions.size());
        return functions;
    }
    
    // ==================== Host Function Implementations ====================
    
    private BlockPos getBlockInFrontPos(TerminalBlockEntity terminal) {
        Direction facing = terminal.getBlockState().getValue(TerminalBlock.FACING);
        return terminal.getBlockPos().relative(facing);
    }
    
    private BlockState getBlockInFront(TerminalBlockEntity terminal) {
        if (terminal.getLevel() == null) return Blocks.AIR.defaultBlockState();
        return terminal.getLevel().getBlockState(getBlockInFrontPos(terminal));
    }
    
    private int hostGetBlockName(TerminalWasmHost host, TerminalBlockEntity terminal,
                                   int bufPtr, int bufLen) {
        if (terminal.getLevel() == null) return ERROR_NOT_AVAILABLE;
        
        BlockState state = getBlockInFront(terminal);
        if (state.is(Blocks.AIR)) {
            return host.writeString(bufPtr, bufLen, "none");
        }
        
        ResourceLocation name = BuiltInRegistries.BLOCK.getKey(state.getBlock());
        return host.writeString(bufPtr, bufLen, name != null ? name.toString() : "unknown");
    }
    
    private int hostGetBlockData(TerminalWasmHost host, TerminalBlockEntity terminal,
                                  int bufPtr, int bufLen) {
        if (terminal.getLevel() == null) return ERROR_NOT_AVAILABLE;
        
        BlockState state = getBlockInFront(terminal);
        if (state.is(Blocks.AIR)) {
            return host.writeString(bufPtr, bufLen, "null");
        }
        
        Level level = terminal.getLevel();
        BlockEntity be = level.getBlockEntity(getBlockInFrontPos(terminal));
        if (be == null) {
            return host.writeString(bufPtr, bufLen, "null");
        }
        
        // Get NBT data and convert to simple JSON representation
        CompoundTag nbt = be.saveWithoutMetadata(level.registryAccess());
        String json = nbtToJson(nbt);
        return host.writeString(bufPtr, bufLen, json);
    }
    
    private int hostGetBlockStates(TerminalWasmHost host, TerminalBlockEntity terminal,
                                    int bufPtr, int bufLen) {
        if (terminal.getLevel() == null) return ERROR_NOT_AVAILABLE;
        
        BlockState state = getBlockInFront(terminal);
        if (state.is(Blocks.AIR)) {
            return host.writeString(bufPtr, bufLen, "null");
        }
        
        Map<String, String> states = new HashMap<>();
        for (Property<?> property : state.getProperties()) {
            states.put(property.getName(), state.getValue(property).toString());
        }
        
        String json = mapToJson(states);
        return host.writeString(bufPtr, bufLen, json);
    }
    
    private int hostIsTileEntity(TerminalBlockEntity terminal) {
        if (terminal.getLevel() == null) return ERROR_NOT_AVAILABLE;
        
        BlockState state = getBlockInFront(terminal);
        if (state.is(Blocks.AIR)) {
            return 0;
        }
        
        BlockEntity be = terminal.getLevel().getBlockEntity(getBlockInFrontPos(terminal));
        return be != null ? 1 : 0;
    }
    
    private String nbtToJson(CompoundTag nbt) {
        // Simple NBT to JSON conversion (not complete, but handles common cases)
        StringBuilder sb = new StringBuilder("{");
        boolean first = true;
        for (String key : nbt.getAllKeys()) {
            if (!first) sb.append(",");
            sb.append("\"").append(escapeJson(key)).append("\":");
            sb.append("\"").append(escapeJson(nbt.get(key).toString())).append("\"");
            first = false;
        }
        sb.append("}");
        return sb.toString();
    }
    
    private String mapToJson(Map<String, String> map) {
        StringBuilder sb = new StringBuilder("{");
        boolean first = true;
        for (Map.Entry<String, String> entry : map.entrySet()) {
            if (!first) sb.append(",");
            sb.append("\"").append(escapeJson(entry.getKey())).append("\":");
            sb.append("\"").append(escapeJson(entry.getValue())).append("\"");
            first = false;
        }
        sb.append("}");
        return sb.toString();
    }
    
    private String escapeJson(String s) {
        return s.replace("\\", "\\\\").replace("\"", "\\\"")
                .replace("\n", "\\n").replace("\r", "\\r");
    }
}
