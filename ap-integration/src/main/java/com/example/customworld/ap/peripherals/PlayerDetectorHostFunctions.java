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
import net.minecraft.server.level.ServerLevel;
import net.minecraft.server.level.ServerPlayer;

import java.nio.charset.StandardCharsets;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import java.util.stream.Collectors;
import java.util.stream.Stream;

/**
 * Provides WASM host functions for player detection functionality.
 * 
 * This mirrors the functionality of Advanced Peripherals' PlayerDetectorPeripheral,
 * exposing it as WASM host functions that can be called from the terminal OS.
 * 
 * Host functions provided:
 * - player_detector_get_online_players(buf_ptr, buf_len) -> bytes_written
 * - player_detector_get_players_in_range(range, buf_ptr, buf_len) -> bytes_written
 * - player_detector_is_player_in_range(range, name_ptr, name_len) -> 0/1
 * - player_detector_get_player_count() -> count
 * - player_detector_get_player_pos(name_ptr, name_len, buf_ptr, buf_len) -> bytes_written
 */
public class PlayerDetectorHostFunctions implements PeripheralHostProvider {
    
    // Error codes
    private static final int ERROR_INVALID_ARGS = -1;
    private static final int ERROR_NOT_AVAILABLE = -2;
    private static final int ERROR_PLAYER_NOT_FOUND = -3;
    
    @Override
    public String getName() {
        return "player_detector";
    }
    
    @Override
    public boolean isAvailable() {
        // Check if Advanced Peripherals is loaded
        try {
            Class.forName("de.srendi.advancedperipherals.AdvancedPeripherals");
            return true;
        } catch (ClassNotFoundException e) {
            APIntegrationLoader.LOGGER.debug("Advanced Peripherals not found, player detector disabled");
            return false;
        }
    }
    
    @Override
    public Map<String, Extern> createHostFunctions(TerminalWasmHost host, List<Func> hostFunctions) {
        Map<String, Extern> functions = new HashMap<>();
        Store<Void> store = host.getStore();
        TerminalBlockEntity terminal = host.getTerminal();
        
        // player_detector_get_online_players(buf_ptr: i32, buf_len: i32) -> i32
        // Returns JSON array of player names, or error code
        Func getOnlinePlayersFunc = new Func(store,
                new FuncType(new Type[]{Type.I32, Type.I32}, new Type[]{Type.I32}),
                (caller, params, results) -> {
                    int bufPtr = params[0].i32();
                    int bufLen = params[1].i32();
                    results[0] = Val.fromI32(hostGetOnlinePlayers(host, bufPtr, bufLen));
                });
        hostFunctions.add(getOnlinePlayersFunc);
        functions.put("player_detector_get_online_players", Extern.fromFunc(getOnlinePlayersFunc));
        
        // player_detector_get_players_in_range(range: i32, buf_ptr: i32, buf_len: i32) -> i32
        // Returns JSON array of player names within range
        Func getPlayersInRangeFunc = new Func(store,
                new FuncType(new Type[]{Type.I32, Type.I32, Type.I32}, new Type[]{Type.I32}),
                (caller, params, results) -> {
                    int range = params[0].i32();
                    int bufPtr = params[1].i32();
                    int bufLen = params[2].i32();
                    results[0] = Val.fromI32(hostGetPlayersInRange(host, terminal, range, bufPtr, bufLen));
                });
        hostFunctions.add(getPlayersInRangeFunc);
        functions.put("player_detector_get_players_in_range", Extern.fromFunc(getPlayersInRangeFunc));
        
        // player_detector_is_player_in_range(range: i32, name_ptr: i32, name_len: i32) -> i32
        // Returns 1 if player is in range, 0 if not, negative for error
        Func isPlayerInRangeFunc = new Func(store,
                new FuncType(new Type[]{Type.I32, Type.I32, Type.I32}, new Type[]{Type.I32}),
                (caller, params, results) -> {
                    int range = params[0].i32();
                    int namePtr = params[1].i32();
                    int nameLen = params[2].i32();
                    results[0] = Val.fromI32(hostIsPlayerInRange(host, terminal, range, namePtr, nameLen));
                });
        hostFunctions.add(isPlayerInRangeFunc);
        functions.put("player_detector_is_player_in_range", Extern.fromFunc(isPlayerInRangeFunc));
        
        // player_detector_get_player_count() -> i32
        // Returns number of online players
        Func getPlayerCountFunc = new Func(store,
                new FuncType(new Type[]{}, new Type[]{Type.I32}),
                (caller, params, results) -> {
                    results[0] = Val.fromI32(hostGetPlayerCount(terminal));
                });
        hostFunctions.add(getPlayerCountFunc);
        functions.put("player_detector_get_player_count", Extern.fromFunc(getPlayerCountFunc));
        
        // player_detector_get_player_pos(name_ptr: i32, name_len: i32, buf_ptr: i32, buf_len: i32) -> i32
        // Returns JSON with player position {x, y, z, dimension}
        Func getPlayerPosFunc = new Func(store,
                new FuncType(new Type[]{Type.I32, Type.I32, Type.I32, Type.I32}, new Type[]{Type.I32}),
                (caller, params, results) -> {
                    int namePtr = params[0].i32();
                    int nameLen = params[1].i32();
                    int bufPtr = params[2].i32();
                    int bufLen = params[3].i32();
                    results[0] = Val.fromI32(hostGetPlayerPos(host, terminal, namePtr, nameLen, bufPtr, bufLen));
                });
        hostFunctions.add(getPlayerPosFunc);
        functions.put("player_detector_get_player_pos", Extern.fromFunc(getPlayerPosFunc));
        
        APIntegrationLoader.LOGGER.debug("Created {} player detector host functions", functions.size());
        return functions;
    }
    
    // ==================== Host Function Implementations ====================
    
    /**
     * Gets all online players and writes their names as a JSON array.
     */
    private int hostGetOnlinePlayers(TerminalWasmHost host, int bufPtr, int bufLen) {
        TerminalBlockEntity terminal = host.getTerminal();
        if (terminal.getLevel() == null || terminal.getLevel().getServer() == null) {
            return ERROR_NOT_AVAILABLE;
        }
        
        List<String> players = terminal.getLevel().getServer().getPlayerList().getPlayers()
                .stream()
                .map(p -> p.getName().getString())
                .collect(Collectors.toList());
        
        String json = toJsonArray(players);
        return host.writeString(bufPtr, bufLen, json);
    }
    
    /**
     * Gets players within the specified range of the terminal.
     */
    private int hostGetPlayersInRange(TerminalWasmHost host, TerminalBlockEntity terminal, 
                                       int range, int bufPtr, int bufLen) {
        if (terminal.getLevel() == null || !(terminal.getLevel() instanceof ServerLevel level)) {
            return ERROR_NOT_AVAILABLE;
        }
        
        BlockPos terminalPos = terminal.getBlockPos();
        
        List<String> playersInRange = level.players().stream()
                .filter(player -> isInRange(terminalPos, player, range))
                .map(player -> player.getName().getString())
                .collect(Collectors.toList());
        
        String json = toJsonArray(playersInRange);
        return host.writeString(bufPtr, bufLen, json);
    }
    
    /**
     * Checks if a specific player is within range.
     */
    private int hostIsPlayerInRange(TerminalWasmHost host, TerminalBlockEntity terminal,
                                     int range, int namePtr, int nameLen) {
        if (terminal.getLevel() == null || !(terminal.getLevel() instanceof ServerLevel level)) {
            return ERROR_NOT_AVAILABLE;
        }
        
        String playerName = host.readString(namePtr, nameLen);
        if (playerName == null) {
            return ERROR_INVALID_ARGS;
        }
        
        ServerPlayer player = level.getServer().getPlayerList().getPlayerByName(playerName);
        if (player == null) {
            return ERROR_PLAYER_NOT_FOUND;
        }
        
        BlockPos terminalPos = terminal.getBlockPos();
        return isInRange(terminalPos, player, range) ? 1 : 0;
    }
    
    /**
     * Gets the total count of online players.
     */
    private int hostGetPlayerCount(TerminalBlockEntity terminal) {
        if (terminal.getLevel() == null || terminal.getLevel().getServer() == null) {
            return ERROR_NOT_AVAILABLE;
        }
        
        return terminal.getLevel().getServer().getPlayerList().getPlayerCount();
    }
    
    /**
     * Gets a player's position as JSON.
     */
    private int hostGetPlayerPos(TerminalWasmHost host, TerminalBlockEntity terminal,
                                  int namePtr, int nameLen, int bufPtr, int bufLen) {
        if (terminal.getLevel() == null || terminal.getLevel().getServer() == null) {
            return ERROR_NOT_AVAILABLE;
        }
        
        String playerName = host.readString(namePtr, nameLen);
        if (playerName == null) {
            return ERROR_INVALID_ARGS;
        }
        
        ServerPlayer player = terminal.getLevel().getServer().getPlayerList().getPlayerByName(playerName);
        if (player == null) {
            return ERROR_PLAYER_NOT_FOUND;
        }
        
        // Build JSON object with player info
        StringBuilder json = new StringBuilder();
        json.append("{");
        json.append("\"x\":").append(player.getX()).append(",");
        json.append("\"y\":").append(player.getY()).append(",");
        json.append("\"z\":").append(player.getZ()).append(",");
        json.append("\"dimension\":\"").append(escapeJson(player.level().dimension().location().toString())).append("\",");
        json.append("\"yaw\":").append(player.getYRot()).append(",");
        json.append("\"pitch\":").append(player.getXRot()).append(",");
        json.append("\"health\":").append(player.getHealth()).append(",");
        json.append("\"maxHealth\":").append(player.getMaxHealth());
        json.append("}");
        
        return host.writeString(bufPtr, bufLen, json.toString());
    }
    
    // ==================== Helper Methods ====================
    
    /**
     * Checks if a player is within range of a position.
     */
    private boolean isInRange(BlockPos pos, ServerPlayer player, int range) {
        if (range < 0) {
            // Negative range means unlimited
            return true;
        }
        double distSq = pos.distToCenterSqr(player.getX(), player.getY(), player.getZ());
        return distSq <= (double) range * range;
    }
    
    /**
     * Converts a list of strings to a JSON array string.
     */
    private String toJsonArray(List<String> items) {
        if (items.isEmpty()) {
            return "[]";
        }
        
        StringBuilder sb = new StringBuilder();
        sb.append("[");
        for (int i = 0; i < items.size(); i++) {
            if (i > 0) {
                sb.append(",");
            }
            sb.append("\"").append(escapeJson(items.get(i))).append("\"");
        }
        sb.append("]");
        return sb.toString();
    }
    
    /**
     * Escapes a string for JSON.
     */
    private String escapeJson(String s) {
        if (s == null) {
            return "";
        }
        return s.replace("\\", "\\\\")
                .replace("\"", "\\\"")
                .replace("\n", "\\n")
                .replace("\r", "\\r")
                .replace("\t", "\\t");
    }
}
