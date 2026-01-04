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
import net.minecraft.network.chat.Component;
import net.minecraft.resources.ResourceKey;
import net.minecraft.server.level.ServerPlayer;
import net.minecraft.world.level.Level;
import net.neoforged.neoforge.server.ServerLifecycleHooks;

import java.util.HashMap;
import java.util.List;
import java.util.Map;

/**
 * Provides WASM host functions for chat messaging.
 * 
 * Mirrors Advanced Peripherals' ChatBoxPeripheral functionality.
 */
public class ChatBoxHostFunctions implements PeripheralHostProvider {
    
    private static final int ERROR_NOT_AVAILABLE = -2;
    private static final int ERROR_PLAYER_NOT_FOUND = -3;
    private static final int MAX_MESSAGE_LENGTH = 512;
    
    @Override
    public String getName() {
        return "chat_box";
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
        
        // chat_send_message(msg_ptr, msg_len) -> i32
        Func sendMessageFunc = new Func(store,
                new FuncType(new Type[]{Type.I32, Type.I32}, new Type[]{Type.I32}),
                (caller, params, results) -> {
                    int msgPtr = params[0].i32();
                    int msgLen = params[1].i32();
                    results[0] = Val.fromI32(hostSendMessage(host, terminal, msgPtr, msgLen));
                });
        hostFunctions.add(sendMessageFunc);
        functions.put("chat_send_message", Extern.fromFunc(sendMessageFunc));
        
        // chat_send_message_to_player(player_ptr, player_len, msg_ptr, msg_len) -> i32
        Func sendToPlayerFunc = new Func(store,
                new FuncType(new Type[]{Type.I32, Type.I32, Type.I32, Type.I32}, new Type[]{Type.I32}),
                (caller, params, results) -> {
                    int playerPtr = params[0].i32();
                    int playerLen = params[1].i32();
                    int msgPtr = params[2].i32();
                    int msgLen = params[3].i32();
                    results[0] = Val.fromI32(hostSendMessageToPlayer(host, terminal, 
                            playerPtr, playerLen, msgPtr, msgLen));
                });
        hostFunctions.add(sendToPlayerFunc);
        functions.put("chat_send_message_to_player", Extern.fromFunc(sendToPlayerFunc));
        
        // chat_send_toast(player_ptr, player_len, title_ptr, title_len, msg_ptr, msg_len) -> i32
        Func sendToastFunc = new Func(store,
                new FuncType(new Type[]{Type.I32, Type.I32, Type.I32, Type.I32, Type.I32, Type.I32}, new Type[]{Type.I32}),
                (caller, params, results) -> {
                    int playerPtr = params[0].i32();
                    int playerLen = params[1].i32();
                    int titlePtr = params[2].i32();
                    int titleLen = params[3].i32();
                    int msgPtr = params[4].i32();
                    int msgLen = params[5].i32();
                    results[0] = Val.fromI32(hostSendToast(host, terminal,
                            playerPtr, playerLen, titlePtr, titleLen, msgPtr, msgLen));
                });
        hostFunctions.add(sendToastFunc);
        functions.put("chat_send_toast", Extern.fromFunc(sendToastFunc));
        
        APIntegrationLoader.LOGGER.debug("Created {} chat box host functions", functions.size());
        return functions;
    }
    
    // ==================== Host Function Implementations ====================
    
    private int hostSendMessage(TerminalWasmHost host, TerminalBlockEntity terminal,
                                 int msgPtr, int msgLen) {
        if (terminal.getLevel() == null || terminal.getLevel().getServer() == null) {
            return ERROR_NOT_AVAILABLE;
        }
        
        String message = host.readString(msgPtr, msgLen);
        if (message == null || message.isEmpty()) {
            return 0;  // Empty message, nothing sent
        }
        
        if (message.length() > MAX_MESSAGE_LENGTH) {
            message = message.substring(0, MAX_MESSAGE_LENGTH);
        }
        
        Component chatMessage = Component.literal("[Terminal] ").append(message);
        ResourceKey<Level> dimension = terminal.getLevel().dimension();
        
        int count = 0;
        for (ServerPlayer player : terminal.getLevel().getServer().getPlayerList().getPlayers()) {
            // Only send to players in same dimension
            if (player.level().dimension() == dimension) {
                player.sendSystemMessage(chatMessage);
                count++;
            }
        }
        
        return count;
    }
    
    private int hostSendMessageToPlayer(TerminalWasmHost host, TerminalBlockEntity terminal,
                                         int playerPtr, int playerLen, int msgPtr, int msgLen) {
        if (terminal.getLevel() == null || terminal.getLevel().getServer() == null) {
            return ERROR_NOT_AVAILABLE;
        }
        
        String playerName = host.readString(playerPtr, playerLen);
        String message = host.readString(msgPtr, msgLen);
        
        if (playerName == null || message == null) {
            return -1;
        }
        
        if (message.length() > MAX_MESSAGE_LENGTH) {
            message = message.substring(0, MAX_MESSAGE_LENGTH);
        }
        
        ServerPlayer player = terminal.getLevel().getServer().getPlayerList().getPlayerByName(playerName);
        if (player == null) {
            return ERROR_PLAYER_NOT_FOUND;
        }
        
        Component chatMessage = Component.literal("[Terminal] ").append(message);
        player.sendSystemMessage(chatMessage);
        return 1;
    }
    
    private int hostSendToast(TerminalWasmHost host, TerminalBlockEntity terminal,
                               int playerPtr, int playerLen, int titlePtr, int titleLen,
                               int msgPtr, int msgLen) {
        if (terminal.getLevel() == null || terminal.getLevel().getServer() == null) {
            return ERROR_NOT_AVAILABLE;
        }
        
        String playerName = host.readString(playerPtr, playerLen);
        String title = host.readString(titlePtr, titleLen);
        String message = host.readString(msgPtr, msgLen);
        
        if (playerName == null || title == null || message == null) {
            return -1;
        }
        
        ServerPlayer player = terminal.getLevel().getServer().getPlayerList().getPlayerByName(playerName);
        if (player == null) {
            return ERROR_PLAYER_NOT_FOUND;
        }
        
        // Toast notifications require client-side packets
        // For simplicity, we'll send as a system message with formatting
        Component toastMessage = Component.literal("[" + title + "] ").append(message);
        player.sendSystemMessage(toastMessage);
        return 1;
    }
}
