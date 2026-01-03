package com.example.customworld.wasm;

import com.example.customworld.CustomWorldMod;
import com.example.customworld.block.TerminalBlockEntity;
import io.github.kawamuray.wasmtime.Engine;
import io.github.kawamuray.wasmtime.Extern;
import io.github.kawamuray.wasmtime.Func;
import io.github.kawamuray.wasmtime.FuncType;
import io.github.kawamuray.wasmtime.Instance;
import io.github.kawamuray.wasmtime.Memory;
import io.github.kawamuray.wasmtime.Store;
import io.github.kawamuray.wasmtime.Val;
import io.github.kawamuray.wasmtime.Val.Type;
import io.github.kawamuray.wasmtime.WasmFunctions;
import io.github.kawamuray.wasmtime.WasmValType;
import io.github.kawamuray.wasmtime.WasmtimeException;

import java.nio.ByteBuffer;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import java.util.Optional;

/**
 * Provides host functions for WASM modules running in a terminal context.
 * Allows WASM modules to write to the terminal display.
 */
public class TerminalWasmHost implements AutoCloseable {
    
    private final TerminalBlockEntity terminal;
    private final Engine engine;
    private final Store<Void> store;
    
    private Instance instance;
    private Memory memory;
    
    // Host functions (need to keep references to prevent GC)
    private final List<Func> hostFunctions = new ArrayList<>();
    private final Map<String, Extern> hostFunctionMap = new HashMap<>();
    
    public TerminalWasmHost(TerminalBlockEntity terminal) {
        this.terminal = terminal;
        // IMPORTANT: Store.withoutData() creates its own Engine internally.
        // We MUST use store.engine() for Module.fromFile(), otherwise we get
        // "cross-Engine instantiation is not currently supported" error!
        this.store = Store.withoutData();
        this.engine = store.engine();
        
        // Create all host functions
        createHostFunctions();
    }
    
    /**
     * Creates all host functions and stores them in a map for lookup by name.
     */
    private void createHostFunctions() {
        // terminal_write(ptr: i32, len: i32) -> i32
        Func terminalWriteFunc = WasmFunctions.wrap(store, WasmValType.I32, WasmValType.I32, WasmValType.I32,
                (Integer ptr, Integer len) -> hostTerminalWrite(ptr, len));
        hostFunctions.add(terminalWriteFunc);
        hostFunctionMap.put("terminal_write", Extern.fromFunc(terminalWriteFunc));
        
        // terminal_clear() -> void
        // Create a function with no parameters and no return values
        Func terminalClearFunc = new Func(store, new FuncType(new Type[]{}, new Type[]{}), 
                (caller, params, results) -> {
                    terminal.clearBuffer();
                });
        hostFunctions.add(terminalClearFunc);
        hostFunctionMap.put("terminal_clear", Extern.fromFunc(terminalClearFunc));
        
        // terminal_set_cursor(x: i32, y: i32) -> void
        // Create a function with two i32 parameters and no return values
        Func terminalSetCursorFunc = new Func(store, 
                new FuncType(new Type[]{Type.I32, Type.I32}, new Type[]{}), 
                (caller, params, results) -> {
                    int x = params[0].i32();
                    int y = params[1].i32();
                    terminal.setCursor(x, y);
                });
        hostFunctions.add(terminalSetCursorFunc);
        hostFunctionMap.put("terminal_set_cursor", Extern.fromFunc(terminalSetCursorFunc));
        
        // terminal_get_width() -> i32
        Func terminalGetWidthFunc = WasmFunctions.wrap(store, WasmValType.I32,
                () -> TerminalBlockEntity.TERMINAL_WIDTH);
        hostFunctions.add(terminalGetWidthFunc);
        hostFunctionMap.put("terminal_get_width", Extern.fromFunc(terminalGetWidthFunc));
        
        // terminal_get_height() -> i32
        Func terminalGetHeightFunc = WasmFunctions.wrap(store, WasmValType.I32,
                () -> TerminalBlockEntity.TERMINAL_HEIGHT);
        hostFunctions.add(terminalGetHeightFunc);
        hostFunctionMap.put("terminal_get_height", Extern.fromFunc(terminalGetHeightFunc));
        
        CustomWorldMod.LOGGER.debug("Created {} host functions", hostFunctions.size());
    }
    
    /**
     * Creates the imports list in the order required by the module.
     * Uses the module's import list to determine the correct order.
     */
    private List<Extern> createImportsForModule(io.github.kawamuray.wasmtime.Module module) {
        List<Extern> imports = new ArrayList<>();
        
        // Get the module's imports and iterate in order
        var moduleImports = module.imports();
        
        for (var importType : moduleImports) {
            String moduleName = importType.module();
            String name = importType.name();
            
            CustomWorldMod.LOGGER.debug("Module requires import: {}::{}", moduleName, name);
            
            // Look up the host function by name
            if (hostFunctionMap.containsKey(name)) {
                imports.add(hostFunctionMap.get(name));
                CustomWorldMod.LOGGER.debug("  -> Matched to host function: {}", name);
            } else {
                // Unknown import - this will cause instantiation to fail
                // Log a warning so we know what's missing
                CustomWorldMod.LOGGER.warn("Unknown WASM import: {}::{} (type: {})", 
                        moduleName, name, importType.type());
                
                // Try to provide a stub based on the import type
                Extern stub = createStubImport(importType);
                if (stub != null) {
                    imports.add(stub);
                    CustomWorldMod.LOGGER.debug("  -> Created stub for: {}", name);
                }
            }
        }
        
        return imports;
    }
    
    /**
     * Creates a stub import for unknown imports (like __stack_pointer).
     */
    private Extern createStubImport(io.github.kawamuray.wasmtime.ImportType importType) {
        // For now, we can't easily create stubs without knowing the exact type
        // The module will fail to instantiate if there are unknown imports
        return null;
    }
    
    /**
     * Host function implementation: writes a string from WASM memory to the terminal.
     * 
     * @param ptr Pointer to the string in WASM memory
     * @param len Length of the string in bytes
     * @return Number of bytes written, or -1 on error
     */
    private int hostTerminalWrite(int ptr, int len) {
        if (memory == null) {
            CustomWorldMod.LOGGER.error("WASM memory not initialized");
            return -1;
        }
        
        if (len <= 0 || len > 4096) {  // Limit max string length for safety
            return -1;
        }
        
        try {
            ByteBuffer buffer = memory.buffer(store);
            byte[] bytes = new byte[len];
            
            // Read bytes from WASM memory
            buffer.position(ptr);
            buffer.get(bytes, 0, len);
            
            // Convert to string and write to terminal
            String text = new String(bytes, StandardCharsets.UTF_8);
            terminal.write(text);
            
            CustomWorldMod.LOGGER.debug("WASM wrote to terminal: {}", text);
            return len;
            
        } catch (Exception e) {
            CustomWorldMod.LOGGER.error("Error reading from WASM memory", e);
            return -1;
        }
    }
    
    /**
     * Loads and instantiates a WASM module.
     * Queries the module's imports and provides them in the correct order.
     * 
     * @param fileName The name of the WASM file (with or without .wasm extension)
     * @throws WasmManager.WasmExecutionException If loading fails
     */
    public void loadModule(String fileName) throws WasmManager.WasmExecutionException {
        // Ensure .wasm extension
        if (!fileName.endsWith(".wasm")) {
            fileName = fileName + ".wasm";
        }
        
        Path wasmFile = WasmManager.getWasmBinPath().resolve(fileName);
        
        if (!Files.exists(wasmFile)) {
            throw new WasmManager.WasmExecutionException("WASM file not found: " + wasmFile.toAbsolutePath());
        }
        
        try {
            io.github.kawamuray.wasmtime.Module module = io.github.kawamuray.wasmtime.Module.fromFile(engine, wasmFile.toString());
            
            CustomWorldMod.LOGGER.info("Loading WASM module: {}", fileName);
            
            // Create imports in the order required by the module
            List<Extern> imports = createImportsForModule(module);
            
            CustomWorldMod.LOGGER.info("Providing {} imports to WASM module", imports.size());
            
            // Create instance with imports
            instance = new Instance(store, module, imports);
            
            // Get the memory export for reading strings
            Optional<Memory> memoryOpt = instance.getMemory(store, "memory");
            if (memoryOpt.isPresent()) {
                memory = memoryOpt.get();
                CustomWorldMod.LOGGER.debug("Got WASM memory export");
            } else {
                CustomWorldMod.LOGGER.warn("WASM module does not export 'memory'");
            }
            
            CustomWorldMod.LOGGER.info("Successfully loaded WASM module: {}", fileName);
            
        } catch (WasmtimeException e) {
            throw new WasmManager.WasmExecutionException("Failed to load WASM module: " + e.getMessage(), e);
        }
    }
    
    /**
     * Executes a function from the loaded WASM module.
     * 
     * @param functionName The name of the function to execute
     * @param params Parameters to pass to the function
     * @return The result of the function
     * @throws WasmManager.WasmExecutionException If execution fails
     */
    public WasmManager.WasmResult executeFunction(String functionName, Val... params) 
            throws WasmManager.WasmExecutionException {
        
        if (instance == null) {
            throw new WasmManager.WasmExecutionException("No WASM module loaded");
        }
        
        try {
            Func func = instance.getFunc(store, functionName)
                    .orElseThrow(() -> new WasmManager.WasmExecutionException(
                            "Function '" + functionName + "' not found in module"));
            
            Val[] results = func.call(store, params);
            return new WasmManager.WasmResult(results);
            
        } catch (WasmtimeException e) {
            throw new WasmManager.WasmExecutionException("WASM execution error: " + e.getMessage(), e);
        }
    }
    
    /**
     * Executes the main/start function if it exists.
     */
    public void executeMain() throws WasmManager.WasmExecutionException {
        if (instance == null) {
            throw new WasmManager.WasmExecutionException("No WASM module loaded");
        }
        
        // Try common entry point names
        String[] entryPoints = {"main", "_start", "start", "init"};
        
        for (String entryPoint : entryPoints) {
            Optional<Func> funcOpt = instance.getFunc(store, entryPoint);
            if (funcOpt.isPresent()) {
                try {
                    funcOpt.get().call(store);
                    CustomWorldMod.LOGGER.info("Executed WASM entry point: {}", entryPoint);
                    return;
                } catch (WasmtimeException e) {
                    throw new WasmManager.WasmExecutionException(
                            "Error executing " + entryPoint + ": " + e.getMessage(), e);
                }
            }
        }
        
        CustomWorldMod.LOGGER.warn("No entry point found in WASM module");
    }
    
    /**
     * Writes a line to the WASM module's input buffer (if the module supports it).
     * This is called when the user enters input in the terminal.
     * 
     * @param line The input line from the user
     */
    public void sendInput(String line) {
        // Look for an input handler function in the WASM module
        if (instance == null) {
            return;
        }
        
        Optional<Func> inputHandler = instance.getFunc(store, "on_input");
        if (inputHandler.isEmpty()) {
            inputHandler = instance.getFunc(store, "handle_input");
        }
        
        if (inputHandler.isPresent() && memory != null) {
            try {
                // Write the input string to WASM memory
                byte[] bytes = line.getBytes(StandardCharsets.UTF_8);
                ByteBuffer buffer = memory.buffer(store);
                
                // Use a fixed input buffer location (at address 0x10000)
                int inputBufferAddr = 0x10000;
                buffer.position(inputBufferAddr);
                buffer.put(bytes);
                
                // Call the input handler with pointer and length
                inputHandler.get().call(store, Val.fromI32(inputBufferAddr), Val.fromI32(bytes.length));
                
            } catch (Exception e) {
                CustomWorldMod.LOGGER.error("Error sending input to WASM", e);
            }
        }
    }
    
    @Override
    public void close() {
        if (instance != null) {
            instance.close();
        }
        for (Func func : hostFunctions) {
            func.close();
        }
        hostFunctions.clear();
        hostFunctionMap.clear();
        // Only close the store - the engine is owned by the store
        store.close();
    }
}
