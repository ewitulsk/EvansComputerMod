package com.example.evanscomputermod.command;

import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.wasm.WasmManager;
import com.mojang.brigadier.CommandDispatcher;
import com.mojang.brigadier.arguments.StringArgumentType;
import com.mojang.brigadier.context.CommandContext;
import com.mojang.brigadier.suggestion.SuggestionProvider;
import net.minecraft.commands.CommandSourceStack;
import net.minecraft.commands.Commands;
import net.minecraft.commands.SharedSuggestionProvider;
import net.minecraft.network.chat.Component;
import net.minecraft.server.permissions.PermissionCheck;
import net.minecraft.server.permissions.Permissions;

import java.util.ArrayList;
import java.util.Arrays;
import java.util.List;

/**
 * Command to execute WASM functions from the wasm-bin directory.
 * 
 * Usage: /wasm <file> <function> [parameters...]
 * 
 * Examples:
 *   /wasm add.wasm add 5 3
 *   /wasm calculator multiply 10 20
 *   /wasm mymodule compute 1.5 2.5
 */
public class WasmCommand {
    
    private static final SuggestionProvider<CommandSourceStack> WASM_FILE_SUGGESTIONS = 
        (context, builder) -> SharedSuggestionProvider.suggest(
            WasmManager.listWasmFiles().stream()
                .map(f -> f.replace(".wasm", "")),
            builder
        );
    
    public static void register(CommandDispatcher<CommandSourceStack> dispatcher) {
        dispatcher.register(
            Commands.literal("wasm")
                .requires(Commands.hasPermission(new PermissionCheck.Require(Permissions.COMMANDS_GAMEMASTER))) // Requires gamemaster level
                .then(Commands.argument("file", StringArgumentType.word())
                    .suggests(WASM_FILE_SUGGESTIONS)
                    .then(Commands.argument("function", StringArgumentType.word())
                        // Without parameters
                        .executes(context -> executeWasm(context, List.of()))
                        // With parameters (as a single greedy string that we'll parse)
                        .then(Commands.argument("params", StringArgumentType.greedyString())
                            .executes(context -> executeWasm(
                                context, 
                                parseParams(StringArgumentType.getString(context, "params"))
                            ))
                        )
                    )
                )
                // Subcommand to list available WASM files
                .then(Commands.literal("list")
                    .executes(WasmCommand::listWasmFiles)
                )
        );
        
        EvansComputerMod.LOGGER.info("Registered WASM command");
    }
    
    private static int executeWasm(CommandContext<CommandSourceStack> context, List<String> params) {
        CommandSourceStack source = context.getSource();
        String fileName = StringArgumentType.getString(context, "file");
        String functionName = StringArgumentType.getString(context, "function");
        
        source.sendSystemMessage(Component.literal(
            "§7Executing WASM: §f" + fileName + "§7::§f" + functionName + 
            "§7(" + String.join(", ", params) + ")§7..."
        ));
        
        try {
            WasmManager.WasmResult result = WasmManager.executeFunction(fileName, functionName, params);
            
            source.sendSuccess(() -> Component.literal(
                "§aWASM Result: §f" + result.toString()
            ), false);
            
            EvansComputerMod.LOGGER.info("WASM execution successful: {}::{} = {}", 
                fileName, functionName, result);
            
            return 1;
            
        } catch (WasmManager.WasmExecutionException e) {
            source.sendFailure(Component.literal("§cWASM Error: §f" + e.getMessage()));
            EvansComputerMod.LOGGER.error("WASM execution failed", e);
            return 0;
        }
    }
    
    private static int listWasmFiles(CommandContext<CommandSourceStack> context) {
        CommandSourceStack source = context.getSource();
        List<String> files = WasmManager.listWasmFiles();
        
        if (files.isEmpty()) {
            source.sendSystemMessage(Component.literal(
                "§7No WASM files found in §fwasm-bin/§7 directory.\n" +
                "§7Place §f.wasm§7 files there to use them."
            ));
        } else {
            StringBuilder sb = new StringBuilder();
            sb.append("§aAvailable WASM files (§f").append(files.size()).append("§a):\n");
            for (String file : files) {
                sb.append("§7 - §f").append(file).append("\n");
            }
            source.sendSystemMessage(Component.literal(sb.toString().trim()));
        }
        
        return 1;
    }
    
    /**
     * Parses a space-separated parameter string into a list.
     * Handles quoted strings for parameters with spaces.
     */
    private static List<String> parseParams(String paramsString) {
        if (paramsString == null || paramsString.trim().isEmpty()) {
            return List.of();
        }
        
        List<String> params = new ArrayList<>();
        StringBuilder current = new StringBuilder();
        boolean inQuotes = false;
        
        for (char c : paramsString.toCharArray()) {
            if (c == '"') {
                inQuotes = !inQuotes;
            } else if (c == ' ' && !inQuotes) {
                if (current.length() > 0) {
                    params.add(current.toString());
                    current = new StringBuilder();
                }
            } else {
                current.append(c);
            }
        }
        
        if (current.length() > 0) {
            params.add(current.toString());
        }
        
        return params;
    }
}
