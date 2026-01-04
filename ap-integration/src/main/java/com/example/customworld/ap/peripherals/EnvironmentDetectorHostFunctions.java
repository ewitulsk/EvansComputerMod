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
import net.minecraft.resources.ResourceKey;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.util.Mth;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.entity.LivingEntity;
import net.minecraft.world.level.ChunkPos;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.LightLayer;
import net.minecraft.world.level.WorldGenLevel;
import net.minecraft.world.level.biome.Biome;
import net.minecraft.world.level.levelgen.WorldgenRandom;
import net.minecraft.world.phys.AABB;
import net.neoforged.neoforge.server.ServerLifecycleHooks;

import java.util.ArrayList;
import java.util.HashMap;
import java.util.HashSet;
import java.util.List;
import java.util.Map;
import java.util.Optional;
import java.util.Set;

/**
 * Provides WASM host functions for environment detection.
 * 
 * Mirrors Advanced Peripherals' EnvironmentDetectorPeripheral functionality.
 */
public class EnvironmentDetectorHostFunctions implements PeripheralHostProvider {
    
    private static final int ERROR_NOT_AVAILABLE = -2;
    
    @Override
    public String getName() {
        return "environment_detector";
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
        
        // environment_get_biome(buf_ptr, buf_len) -> i32
        Func getBiomeFunc = new Func(store,
                new FuncType(new Type[]{Type.I32, Type.I32}, new Type[]{Type.I32}),
                (caller, params, results) -> {
                    int bufPtr = params[0].i32();
                    int bufLen = params[1].i32();
                    results[0] = Val.fromI32(hostGetBiome(host, terminal, bufPtr, bufLen));
                });
        hostFunctions.add(getBiomeFunc);
        functions.put("environment_get_biome", Extern.fromFunc(getBiomeFunc));
        
        // environment_get_time() -> i64
        Func getTimeFunc = new Func(store,
                new FuncType(new Type[]{}, new Type[]{Type.I64}),
                (caller, params, results) -> {
                    results[0] = Val.fromI64(hostGetTime(terminal));
                });
        hostFunctions.add(getTimeFunc);
        functions.put("environment_get_time", Extern.fromFunc(getTimeFunc));
        
        // environment_get_moon_id() -> i32
        Func getMoonIdFunc = new Func(store,
                new FuncType(new Type[]{}, new Type[]{Type.I32}),
                (caller, params, results) -> {
                    results[0] = Val.fromI32(hostGetMoonId(terminal));
                });
        hostFunctions.add(getMoonIdFunc);
        functions.put("environment_get_moon_id", Extern.fromFunc(getMoonIdFunc));
        
        // environment_get_moon_name(buf_ptr, buf_len) -> i32
        Func getMoonNameFunc = new Func(store,
                new FuncType(new Type[]{Type.I32, Type.I32}, new Type[]{Type.I32}),
                (caller, params, results) -> {
                    int bufPtr = params[0].i32();
                    int bufLen = params[1].i32();
                    results[0] = Val.fromI32(hostGetMoonName(host, terminal, bufPtr, bufLen));
                });
        hostFunctions.add(getMoonNameFunc);
        functions.put("environment_get_moon_name", Extern.fromFunc(getMoonNameFunc));
        
        // environment_is_raining() -> i32
        Func isRainingFunc = new Func(store,
                new FuncType(new Type[]{}, new Type[]{Type.I32}),
                (caller, params, results) -> {
                    results[0] = Val.fromI32(hostIsRaining(terminal));
                });
        hostFunctions.add(isRainingFunc);
        functions.put("environment_is_raining", Extern.fromFunc(isRainingFunc));
        
        // environment_is_thunder() -> i32
        Func isThunderFunc = new Func(store,
                new FuncType(new Type[]{}, new Type[]{Type.I32}),
                (caller, params, results) -> {
                    results[0] = Val.fromI32(hostIsThunder(terminal));
                });
        hostFunctions.add(isThunderFunc);
        functions.put("environment_is_thunder", Extern.fromFunc(isThunderFunc));
        
        // environment_is_sunny() -> i32
        Func isSunnyFunc = new Func(store,
                new FuncType(new Type[]{}, new Type[]{Type.I32}),
                (caller, params, results) -> {
                    results[0] = Val.fromI32(hostIsSunny(terminal));
                });
        hostFunctions.add(isSunnyFunc);
        functions.put("environment_is_sunny", Extern.fromFunc(isSunnyFunc));
        
        // environment_get_dimension(buf_ptr, buf_len) -> i32
        Func getDimensionFunc = new Func(store,
                new FuncType(new Type[]{Type.I32, Type.I32}, new Type[]{Type.I32}),
                (caller, params, results) -> {
                    int bufPtr = params[0].i32();
                    int bufLen = params[1].i32();
                    results[0] = Val.fromI32(hostGetDimension(host, terminal, bufPtr, bufLen));
                });
        hostFunctions.add(getDimensionFunc);
        functions.put("environment_get_dimension", Extern.fromFunc(getDimensionFunc));
        
        // environment_list_dimensions(buf_ptr, buf_len) -> i32
        Func listDimensionsFunc = new Func(store,
                new FuncType(new Type[]{Type.I32, Type.I32}, new Type[]{Type.I32}),
                (caller, params, results) -> {
                    int bufPtr = params[0].i32();
                    int bufLen = params[1].i32();
                    results[0] = Val.fromI32(hostListDimensions(host, bufPtr, bufLen));
                });
        hostFunctions.add(listDimensionsFunc);
        functions.put("environment_list_dimensions", Extern.fromFunc(listDimensionsFunc));
        
        // environment_get_sky_light_level() -> i32
        Func getSkyLightFunc = new Func(store,
                new FuncType(new Type[]{}, new Type[]{Type.I32}),
                (caller, params, results) -> {
                    results[0] = Val.fromI32(hostGetSkyLightLevel(terminal));
                });
        hostFunctions.add(getSkyLightFunc);
        functions.put("environment_get_sky_light_level", Extern.fromFunc(getSkyLightFunc));
        
        // environment_get_block_light_level() -> i32
        Func getBlockLightFunc = new Func(store,
                new FuncType(new Type[]{}, new Type[]{Type.I32}),
                (caller, params, results) -> {
                    results[0] = Val.fromI32(hostGetBlockLightLevel(terminal));
                });
        hostFunctions.add(getBlockLightFunc);
        functions.put("environment_get_block_light_level", Extern.fromFunc(getBlockLightFunc));
        
        // environment_get_day_light_level() -> i32
        Func getDayLightFunc = new Func(store,
                new FuncType(new Type[]{}, new Type[]{Type.I32}),
                (caller, params, results) -> {
                    results[0] = Val.fromI32(hostGetDayLightLevel(terminal));
                });
        hostFunctions.add(getDayLightFunc);
        functions.put("environment_get_day_light_level", Extern.fromFunc(getDayLightFunc));
        
        // environment_is_slime_chunk() -> i32
        Func isSlimeChunkFunc = new Func(store,
                new FuncType(new Type[]{}, new Type[]{Type.I32}),
                (caller, params, results) -> {
                    results[0] = Val.fromI32(hostIsSlimeChunk(terminal));
                });
        hostFunctions.add(isSlimeChunkFunc);
        functions.put("environment_is_slime_chunk", Extern.fromFunc(isSlimeChunkFunc));
        
        // environment_scan_entities(radius, buf_ptr, buf_len) -> i32
        Func scanEntitiesFunc = new Func(store,
                new FuncType(new Type[]{Type.I32, Type.I32, Type.I32}, new Type[]{Type.I32}),
                (caller, params, results) -> {
                    int radius = params[0].i32();
                    int bufPtr = params[1].i32();
                    int bufLen = params[2].i32();
                    results[0] = Val.fromI32(hostScanEntities(host, terminal, radius, bufPtr, bufLen));
                });
        hostFunctions.add(scanEntitiesFunc);
        functions.put("environment_scan_entities", Extern.fromFunc(scanEntitiesFunc));
        
        APIntegrationLoader.LOGGER.debug("Created {} environment detector host functions", functions.size());
        return functions;
    }
    
    // ==================== Host Function Implementations ====================
    
    private int hostGetBiome(TerminalWasmHost host, TerminalBlockEntity terminal, int bufPtr, int bufLen) {
        if (terminal.getLevel() == null) return ERROR_NOT_AVAILABLE;
        
        Optional<ResourceKey<Biome>> biome = terminal.getLevel().getBiome(terminal.getBlockPos()).unwrapKey();
        String biomeName = biome.map(b -> b.location().toString()).orElse("unknown");
        return host.writeString(bufPtr, bufLen, biomeName);
    }
    
    private long hostGetTime(TerminalBlockEntity terminal) {
        if (terminal.getLevel() == null) return ERROR_NOT_AVAILABLE;
        return terminal.getLevel().getDayTime();
    }
    
    private int hostGetMoonId(TerminalBlockEntity terminal) {
        if (terminal.getLevel() == null) return ERROR_NOT_AVAILABLE;
        if (terminal.getLevel().dimension() != Level.OVERWORLD) return -1;
        return terminal.getLevel().getMoonPhase();
    }
    
    private int hostGetMoonName(TerminalWasmHost host, TerminalBlockEntity terminal, int bufPtr, int bufLen) {
        if (terminal.getLevel() == null) return ERROR_NOT_AVAILABLE;
        if (terminal.getLevel().dimension() != Level.OVERWORLD) {
            return host.writeString(bufPtr, bufLen, "Moon.exe not found...");
        }
        
        String moonName = switch (terminal.getLevel().getMoonPhase()) {
            case 0 -> "Full moon";
            case 1 -> "Waning gibbous";
            case 2 -> "Third quarter";
            case 3 -> "Waning crescent";
            case 4 -> "New moon";
            case 5 -> "Waxing crescent";
            case 6 -> "First quarter";
            case 7 -> "Waxing gibbous";
            default -> "Unknown";
        };
        return host.writeString(bufPtr, bufLen, moonName);
    }
    
    private int hostIsRaining(TerminalBlockEntity terminal) {
        if (terminal.getLevel() == null) return ERROR_NOT_AVAILABLE;
        return terminal.getLevel().getRainLevel(0) > 0 ? 1 : 0;
    }
    
    private int hostIsThunder(TerminalBlockEntity terminal) {
        if (terminal.getLevel() == null) return ERROR_NOT_AVAILABLE;
        return terminal.getLevel().getThunderLevel(0) > 0 ? 1 : 0;
    }
    
    private int hostIsSunny(TerminalBlockEntity terminal) {
        if (terminal.getLevel() == null) return ERROR_NOT_AVAILABLE;
        Level level = terminal.getLevel();
        return (level.getThunderLevel(0) < 1 && level.getRainLevel(0) < 1) ? 1 : 0;
    }
    
    private int hostGetDimension(TerminalWasmHost host, TerminalBlockEntity terminal, int bufPtr, int bufLen) {
        if (terminal.getLevel() == null) return ERROR_NOT_AVAILABLE;
        String dimension = terminal.getLevel().dimension().location().toString();
        return host.writeString(bufPtr, bufLen, dimension);
    }
    
    private int hostListDimensions(TerminalWasmHost host, int bufPtr, int bufLen) {
        var server = ServerLifecycleHooks.getCurrentServer();
        if (server == null) return ERROR_NOT_AVAILABLE;
        
        Set<String> dimensions = new HashSet<>();
        server.getAllLevels().forEach(level -> 
            dimensions.add(level.dimension().location().toString()));
        
        StringBuilder json = new StringBuilder("[");
        boolean first = true;
        for (String dim : dimensions) {
            if (!first) json.append(",");
            json.append("\"").append(dim).append("\"");
            first = false;
        }
        json.append("]");
        
        return host.writeString(bufPtr, bufLen, json.toString());
    }
    
    private int hostGetSkyLightLevel(TerminalBlockEntity terminal) {
        if (terminal.getLevel() == null) return ERROR_NOT_AVAILABLE;
        return terminal.getLevel().getBrightness(LightLayer.SKY, terminal.getBlockPos().above());
    }
    
    private int hostGetBlockLightLevel(TerminalBlockEntity terminal) {
        if (terminal.getLevel() == null) return ERROR_NOT_AVAILABLE;
        return terminal.getLevel().getBrightness(LightLayer.BLOCK, terminal.getBlockPos().above());
    }
    
    private int hostGetDayLightLevel(TerminalBlockEntity terminal) {
        if (terminal.getLevel() == null) return ERROR_NOT_AVAILABLE;
        Level level = terminal.getLevel();
        BlockPos pos = terminal.getBlockPos().above();
        
        int skyLight = level.getBrightness(LightLayer.SKY, pos) - level.getSkyDarken();
        float sunAngle = level.getSunAngle(1.0F);
        
        if (skyLight > 0) {
            float adjustment = sunAngle < (float) Math.PI ? 0.0F : ((float) Math.PI * 2F);
            sunAngle = sunAngle + (adjustment - sunAngle) * 0.2F;
            skyLight = Math.round(skyLight * Mth.cos(sunAngle));
        }
        
        return Mth.clamp(skyLight, 0, 15);
    }
    
    private int hostIsSlimeChunk(TerminalBlockEntity terminal) {
        if (terminal.getLevel() == null || !(terminal.getLevel() instanceof WorldGenLevel worldGen)) {
            return ERROR_NOT_AVAILABLE;
        }
        
        ChunkPos chunkPos = new ChunkPos(terminal.getBlockPos());
        return WorldgenRandom.seedSlimeChunk(chunkPos.x, chunkPos.z, worldGen.getSeed(), 987234911L)
                .nextInt(10) == 0 ? 1 : 0;
    }
    
    private int hostScanEntities(TerminalWasmHost host, TerminalBlockEntity terminal, 
                                  int radius, int bufPtr, int bufLen) {
        if (terminal.getLevel() == null) return ERROR_NOT_AVAILABLE;
        if (radius < 1 || radius > 64) return -1; // Limit radius for performance
        
        BlockPos pos = terminal.getBlockPos();
        AABB box = new AABB(pos).inflate(radius);
        
        List<Map<String, Object>> entities = new ArrayList<>();
        terminal.getLevel().getEntities((Entity) null, box, e -> e instanceof LivingEntity)
            .forEach(entity -> {
                Map<String, Object> data = new HashMap<>();
                data.put("name", entity.getName().getString());
                data.put("type", entity.getType().toShortString());
                data.put("x", entity.getX() - pos.getX());
                data.put("y", entity.getY() - pos.getY());
                data.put("z", entity.getZ() - pos.getZ());
                if (entity instanceof LivingEntity living) {
                    data.put("health", living.getHealth());
                    data.put("maxHealth", living.getMaxHealth());
                }
                entities.add(data);
            });
        
        String json = toJsonArray(entities);
        return host.writeString(bufPtr, bufLen, json);
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
    
    private String toJsonObject(Map<String, Object> map) {
        StringBuilder sb = new StringBuilder("{");
        boolean first = true;
        for (Map.Entry<String, Object> entry : map.entrySet()) {
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
        return s.replace("\\", "\\\\").replace("\"", "\\\"")
                .replace("\n", "\\n").replace("\r", "\\r");
    }
}
