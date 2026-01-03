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

import java.io.IOException;
import java.nio.ByteBuffer;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.StandardOpenOption;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import java.util.Optional;
import java.util.UUID;
import java.util.stream.Collectors;

/**
 * Provides host functions for WASM modules running in a terminal context.
 * Allows WASM modules to write to the terminal display and access the file system.
 */
public class TerminalWasmHost implements AutoCloseable {
    
    // Directory for storing computer files (in game directory)
    private static final String COMPUTER_DATA_FOLDER = "computer-data";
    
    private final TerminalBlockEntity terminal;
    private final Engine engine;
    private final Store<Void> store;
    private final Path computerStoragePath;
    
    private Instance instance;
    private Memory memory;
    
    // Flag to indicate the WASM module has crashed and should not be used
    private volatile boolean faulted = false;
    
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
        
        // Set up computer storage directory
        UUID computerId = terminal.getComputerId();
        this.computerStoragePath = Path.of(COMPUTER_DATA_FOLDER, computerId.toString());
        try {
            Files.createDirectories(computerStoragePath);
            CustomWorldMod.LOGGER.info("Computer storage path: {}", computerStoragePath.toAbsolutePath());
        } catch (IOException e) {
            CustomWorldMod.LOGGER.error("Failed to create computer storage directory", e);
        }
        
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
        
        // === File System Host Functions ===
        
        // file_write(path_ptr, path_len, data_ptr, data_len) -> bytes_written or -1
        Func fileWriteFunc = new Func(store,
                new FuncType(new Type[]{Type.I32, Type.I32, Type.I32, Type.I32}, new Type[]{Type.I32}),
                (caller, params, results) -> {
                    int pathPtr = params[0].i32();
                    int pathLen = params[1].i32();
                    int dataPtr = params[2].i32();
                    int dataLen = params[3].i32();
                    results[0] = Val.fromI32(hostFileWrite(pathPtr, pathLen, dataPtr, dataLen));
                });
        hostFunctions.add(fileWriteFunc);
        hostFunctionMap.put("file_write", Extern.fromFunc(fileWriteFunc));
        
        // file_read(path_ptr, path_len, buf_ptr, buf_len) -> bytes_read or -1
        Func fileReadFunc = new Func(store,
                new FuncType(new Type[]{Type.I32, Type.I32, Type.I32, Type.I32}, new Type[]{Type.I32}),
                (caller, params, results) -> {
                    int pathPtr = params[0].i32();
                    int pathLen = params[1].i32();
                    int bufPtr = params[2].i32();
                    int bufLen = params[3].i32();
                    results[0] = Val.fromI32(hostFileRead(pathPtr, pathLen, bufPtr, bufLen));
                });
        hostFunctions.add(fileReadFunc);
        hostFunctionMap.put("file_read", Extern.fromFunc(fileReadFunc));
        
        // file_size(path_ptr, path_len) -> file size or -1
        Func fileSizeFunc = new Func(store,
                new FuncType(new Type[]{Type.I32, Type.I32}, new Type[]{Type.I32}),
                (caller, params, results) -> {
                    int pathPtr = params[0].i32();
                    int pathLen = params[1].i32();
                    results[0] = Val.fromI32(hostFileSize(pathPtr, pathLen));
                });
        hostFunctions.add(fileSizeFunc);
        hostFunctionMap.put("file_size", Extern.fromFunc(fileSizeFunc));
        
        // file_exists(path_ptr, path_len) -> 1 if exists, 0 if not
        Func fileExistsFunc = new Func(store,
                new FuncType(new Type[]{Type.I32, Type.I32}, new Type[]{Type.I32}),
                (caller, params, results) -> {
                    int pathPtr = params[0].i32();
                    int pathLen = params[1].i32();
                    results[0] = Val.fromI32(hostFileExists(pathPtr, pathLen));
                });
        hostFunctions.add(fileExistsFunc);
        hostFunctionMap.put("file_exists", Extern.fromFunc(fileExistsFunc));
        
        // file_delete(path_ptr, path_len) -> 1 on success, 0 on failure
        Func fileDeleteFunc = new Func(store,
                new FuncType(new Type[]{Type.I32, Type.I32}, new Type[]{Type.I32}),
                (caller, params, results) -> {
                    int pathPtr = params[0].i32();
                    int pathLen = params[1].i32();
                    results[0] = Val.fromI32(hostFileDelete(pathPtr, pathLen));
                });
        hostFunctions.add(fileDeleteFunc);
        hostFunctionMap.put("file_delete", Extern.fromFunc(fileDeleteFunc));
        
        // file_list(buf_ptr, buf_len) -> bytes written (newline-separated filenames)
        Func fileListFunc = new Func(store,
                new FuncType(new Type[]{Type.I32, Type.I32}, new Type[]{Type.I32}),
                (caller, params, results) -> {
                    int bufPtr = params[0].i32();
                    int bufLen = params[1].i32();
                    results[0] = Val.fromI32(hostFileList(bufPtr, bufLen));
                });
        hostFunctions.add(fileListFunc);
        hostFunctionMap.put("file_list", Extern.fromFunc(fileListFunc));
        
        // === Custom getrandom for getrandom 0.3 ===
        // __getrandom_v03_custom(ptr: i32, len: i32) -> i32
        Func getrandomFunc = new Func(store,
                new FuncType(new Type[]{Type.I32, Type.I32}, new Type[]{Type.I32}),
                (caller, params, results) -> {
                    int ptr = params[0].i32();
                    int len = params[1].i32();
                    results[0] = Val.fromI32(hostGetrandom(ptr, len));
                });
        hostFunctions.add(getrandomFunc);
        hostFunctionMap.put("__getrandom_v03_custom", Extern.fromFunc(getrandomFunc));
        
        // === wasm-bindgen stubs ===
        // These are stubs for wasm-bindgen functions that RustPython's dependencies require.
        // Most of these are never actually called in our non-browser environment.
        createWasmBindgenStubs();
        
        CustomWorldMod.LOGGER.debug("Created {} host functions", hostFunctions.size());
    }
    
    /**
     * Creates stub functions for wasm-bindgen imports.
     * These are required by RustPython's dependencies (chrono, js-sys, etc.)
     * but won't be called in our non-browser WASM environment.
     * 
     * Type signatures from WASM analysis:
     * - type 0: (i32, i32, i32) -> void
     * - type 1: (i32) -> void
     * - type 2: (i32, i32) -> i32
     * - type 3: (i32, i32, i32) -> i32
     * - type 4: (i32) -> i32
     * - type 7: () -> i32
     * - type 16: (i32, i32) -> void
     * - type 20: (i32) -> f64
     * - type 21: () -> f64
     */
    private void createWasmBindgenStubs() {
        // wbindgen core functions
        addStubVoid("__wbindgen_describe", Type.I32);  // type 1: (i32) -> void
        addStubI32Return("__wbindgen_describe_cast", Type.I32, Type.I32);  // type 2: (i32, i32) -> i32
        addStubVoid("__wbindgen_object_drop_ref", Type.I32);  // type 1: (i32) -> void
        addStubI32Return("__wbindgen_object_clone_ref", Type.I32);  // type 4: (i32) -> i32
        
        // Date/time functions - some return f64!
        addStubI32Return("__wbg_new_b2db8aa2650f793a", Type.I32);  // type 4: (i32) -> i32
        addStubF64Return("__wbg_getTimezoneOffset_45389e26d6f46823", 0.0, Type.I32);  // type 20: (i32) -> f64
        addStubI32Return("__wbg_new_0_23cedd11d9b40c9d");  // type 7: () -> i32
        addStubF64Return("__wbg_getTime_ad1e9878a735af08", 0.0, Type.I32);  // type 20: (i32) -> f64
        addStubF64Return("__wbg_now_2c70f2474e348581", (double) System.currentTimeMillis());  // type 21: () -> f64
        
        // Boolean checks - all type 4: (i32) -> i32
        addStubI32ReturnValue("__wbg___wbindgen_is_object_ce774f3490692386", 0, Type.I32);
        addStubI32ReturnValue("__wbg___wbindgen_is_string_704ef9c8fc131030", 0, Type.I32);
        addStubI32ReturnValue("__wbg___wbindgen_is_function_8d400b8b1af978cd", 0, Type.I32);
        addStubI32ReturnValue("__wbg___wbindgen_is_undefined_f6b95eab589e0269", 1, Type.I32);  // Return true (undefined)
        
        // Crypto/random functions
        addStubI32Return("__wbg_crypto_574e78ad8b13b65f", Type.I32);  // type 4: (i32) -> i32
        addStubI32Return("__wbg_msCrypto_a61aeb35a24c1329", Type.I32);  // type 4: (i32) -> i32
        addStubVoid("__wbg_randomFillSync_ac0988aba3254290", Type.I32, Type.I32);  // type 16: (i32, i32) -> void
        addStubVoid("__wbg_getRandomValues_b8f5dbd5f3995a9e", Type.I32, Type.I32);  // type 16: (i32, i32) -> void
        
        // Node.js functions
        addStubI32Return("__wbg_process_dc0fbacc7c1c06f7", Type.I32);  // type 4: (i32) -> i32
        addStubI32Return("__wbg_versions_c01dfd4722a88165", Type.I32);  // type 4: (i32) -> i32
        addStubI32Return("__wbg_node_905d3e251edff8a2", Type.I32);  // type 4: (i32) -> i32
        addStubI32Return("__wbg_require_60cc747a6bc5215a");  // type 7: () -> i32
        
        // Function call stubs
        addStubI32Return("__wbg_call_3020136f7a2d6e44", Type.I32, Type.I32, Type.I32);  // type 3: (i32, i32, i32) -> i32
        addStubI32Return("__wbg_call_abb4ff46ce38be40", Type.I32, Type.I32);  // type 2: (i32, i32) -> i32
        
        // Global/window accessors - type 7: () -> i32
        addStubI32Return("__wbg_static_accessor_GLOBAL_769e6b65d6557335");
        addStubI32Return("__wbg_static_accessor_GLOBAL_THIS_60cf02db4de8e1c1");
        addStubI32Return("__wbg_static_accessor_WINDOW_a8924b26aa92d024");
        addStubI32Return("__wbg_static_accessor_SELF_08f5a74c69739274");
        
        // Array functions
        addStubI32Return("__wbg_new_with_length_aa5eaf41d35235e5", Type.I32);  // type 4: (i32) -> i32
        addStubI32Return("__wbg_subarray_845f2f5bce7d061a", Type.I32, Type.I32, Type.I32);  // type 3: (i32, i32, i32) -> i32
        addStubI32Return("__wbg_length_22ac23eaec9d8053", Type.I32);  // type 4: (i32) -> i32
        
        // Misc functions
        addStubI32Return("__wbg_new_no_args_cb138f77cf6151ee", Type.I32, Type.I32);  // type 2: (i32, i32) -> i32
        addStubVoid("__wbg_prototypesetcall_dfe9b766cdc1f1fd", Type.I32, Type.I32, Type.I32);  // type 0: (i32, i32, i32) -> void
        addStubVoid("__wbg_error_d01e9edc65d6e61f", Type.I32, Type.I32);  // type 16: (i32, i32) -> void
        addStubVoid("__wbg___wbindgen_throw_dd24417ed36fc46e", Type.I32, Type.I32);  // type 16: (i32, i32) -> void
        
        // Externref table functions
        addStubVoid("__wbindgen_externref_table_set_null", Type.I32);  // type 1: (i32) -> void
        addStubI32Return("__wbindgen_externref_table_grow", Type.I32);  // type 4: (i32) -> i32
    }
    
    /**
     * Adds a stub function that takes parameters and returns void.
     */
    private void addStubVoid(String name, Type... paramTypes) {
        Func func = new Func(store, new FuncType(paramTypes, new Type[]{}),
                (caller, params, results) -> {
                    // Do nothing
                });
        hostFunctions.add(func);
        hostFunctionMap.put(name, Extern.fromFunc(func));
    }
    
    /**
     * Adds a stub function that takes parameters and returns i32 (0).
     */
    private void addStubI32Return(String name, Type... paramTypes) {
        addStubI32ReturnValue(name, 0, paramTypes);
    }
    
    /**
     * Adds a stub function that takes parameters and returns a specific i32 value.
     */
    private void addStubI32ReturnValue(String name, int returnValue, Type... paramTypes) {
        Func func = new Func(store, new FuncType(paramTypes, new Type[]{Type.I32}),
                (caller, params, results) -> {
                    results[0] = Val.fromI32(returnValue);
                });
        hostFunctions.add(func);
        hostFunctionMap.put(name, Extern.fromFunc(func));
    }
    
    /**
     * Adds a stub function that takes parameters and returns a specific f64 value.
     */
    private void addStubF64Return(String name, double returnValue, Type... paramTypes) {
        Func func = new Func(store, new FuncType(paramTypes, new Type[]{Type.F64}),
                (caller, params, results) -> {
                    results[0] = Val.fromF64(returnValue);
                });
        hostFunctions.add(func);
        hostFunctionMap.put(name, Extern.fromFunc(func));
    }
    
    /**
     * Host function: provides random bytes for getrandom 0.3.
     */
    private int hostGetrandom(int ptr, int len) {
        if (memory == null || len <= 0 || len > 4096) {
            return -1;
        }
        
        try {
            ByteBuffer buffer = memory.buffer(store);
            buffer.position(ptr);
            
            // Simple xorshift64* PRNG (same as Rust side)
            java.util.Random random = new java.util.Random();
            byte[] bytes = new byte[len];
            random.nextBytes(bytes);
            buffer.put(bytes);
            
            return 0;  // Success
        } catch (Exception e) {
            CustomWorldMod.LOGGER.error("Error in hostGetrandom", e);
            return -1;
        }
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
     * Reads a string from WASM memory.
     */
    private String readStringFromMemory(int ptr, int len) {
        if (memory == null || len <= 0 || len > 4096) {
            return null;
        }
        try {
            ByteBuffer buffer = memory.buffer(store);
            byte[] bytes = new byte[len];
            buffer.position(ptr);
            buffer.get(bytes, 0, len);
            return new String(bytes, StandardCharsets.UTF_8);
        } catch (Exception e) {
            return null;
        }
    }
    
    /**
     * Sanitizes a file path to prevent directory traversal attacks.
     * Returns null if the path is invalid.
     */
    private Path sanitizePath(String filename) {
        if (filename == null || filename.isEmpty()) {
            return null;
        }
        // Reject absolute paths and directory traversal
        if (filename.contains("..") || filename.startsWith("/") || filename.startsWith("\\") ||
            filename.contains(":") || filename.contains("\0")) {
            CustomWorldMod.LOGGER.warn("Rejected unsafe file path: {}", filename);
            return null;
        }
        // Only allow simple filenames (no subdirectories for now)
        Path resolved = computerStoragePath.resolve(filename).normalize();
        // Ensure the resolved path is still within the computer storage
        if (!resolved.startsWith(computerStoragePath)) {
            CustomWorldMod.LOGGER.warn("Path escaped storage directory: {}", filename);
            return null;
        }
        return resolved;
    }
    
    /**
     * Host function: writes data to a file.
     */
    private int hostFileWrite(int pathPtr, int pathLen, int dataPtr, int dataLen) {
        if (memory == null) {
            return -1;
        }
        
        String filename = readStringFromMemory(pathPtr, pathLen);
        Path filePath = sanitizePath(filename);
        if (filePath == null) {
            return -1;
        }
        
        if (dataLen < 0 || dataLen > 1024 * 1024) { // 1MB max file size
            return -1;
        }
        
        try {
            ByteBuffer buffer = memory.buffer(store);
            byte[] data = new byte[dataLen];
            buffer.position(dataPtr);
            buffer.get(data, 0, dataLen);
            
            Files.write(filePath, data, StandardOpenOption.CREATE, StandardOpenOption.TRUNCATE_EXISTING);
            CustomWorldMod.LOGGER.debug("Wrote {} bytes to file: {}", dataLen, filename);
            return dataLen;
        } catch (Exception e) {
            CustomWorldMod.LOGGER.error("Error writing file: {}", filename, e);
            return -1;
        }
    }
    
    /**
     * Host function: reads data from a file.
     */
    private int hostFileRead(int pathPtr, int pathLen, int bufPtr, int bufLen) {
        if (memory == null) {
            return -1;
        }
        
        String filename = readStringFromMemory(pathPtr, pathLen);
        Path filePath = sanitizePath(filename);
        if (filePath == null || !Files.exists(filePath)) {
            return -1;
        }
        
        try {
            byte[] data = Files.readAllBytes(filePath);
            int bytesToRead = Math.min(data.length, bufLen);
            
            ByteBuffer buffer = memory.buffer(store);
            buffer.position(bufPtr);
            buffer.put(data, 0, bytesToRead);
            
            CustomWorldMod.LOGGER.debug("Read {} bytes from file: {}", bytesToRead, filename);
            return bytesToRead;
        } catch (Exception e) {
            CustomWorldMod.LOGGER.error("Error reading file: {}", filename, e);
            return -1;
        }
    }
    
    /**
     * Host function: gets the size of a file.
     */
    private int hostFileSize(int pathPtr, int pathLen) {
        String filename = readStringFromMemory(pathPtr, pathLen);
        Path filePath = sanitizePath(filename);
        if (filePath == null || !Files.exists(filePath)) {
            return -1;
        }
        
        try {
            return (int) Files.size(filePath);
        } catch (Exception e) {
            return -1;
        }
    }
    
    /**
     * Host function: checks if a file exists.
     */
    private int hostFileExists(int pathPtr, int pathLen) {
        String filename = readStringFromMemory(pathPtr, pathLen);
        Path filePath = sanitizePath(filename);
        if (filePath == null) {
            return 0;
        }
        return Files.exists(filePath) ? 1 : 0;
    }
    
    /**
     * Host function: deletes a file.
     */
    private int hostFileDelete(int pathPtr, int pathLen) {
        String filename = readStringFromMemory(pathPtr, pathLen);
        Path filePath = sanitizePath(filename);
        if (filePath == null || !Files.exists(filePath)) {
            return 0;
        }
        
        try {
            Files.delete(filePath);
            CustomWorldMod.LOGGER.debug("Deleted file: {}", filename);
            return 1;
        } catch (Exception e) {
            CustomWorldMod.LOGGER.error("Error deleting file: {}", filename, e);
            return 0;
        }
    }
    
    /**
     * Host function: lists all files in the computer's storage.
     */
    private int hostFileList(int bufPtr, int bufLen) {
        if (memory == null) {
            return -1;
        }
        
        try {
            if (!Files.exists(computerStoragePath)) {
                return 0;
            }
            
            String fileList = Files.list(computerStoragePath)
                    .filter(Files::isRegularFile)
                    .map(p -> p.getFileName().toString())
                    .collect(Collectors.joining("\n"));
            
            byte[] data = fileList.getBytes(StandardCharsets.UTF_8);
            int bytesToWrite = Math.min(data.length, bufLen);
            
            ByteBuffer buffer = memory.buffer(store);
            buffer.position(bufPtr);
            buffer.put(data, 0, bytesToWrite);
            
            return bytesToWrite;
        } catch (Exception e) {
            CustomWorldMod.LOGGER.error("Error listing files", e);
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
     * Checks if the WASM module has faulted and should not be used.
     */
    public boolean isFaulted() {
        return faulted;
    }
    
    /**
     * Writes a line to the WASM module's input buffer (if the module supports it).
     * This is called when the user enters input in the terminal.
     * 
     * @param line The input line from the user
     */
    public void sendInput(String line) {
        // Look for an input handler function in the WASM module
        if (instance == null || faulted) {
            if (faulted) {
                terminal.write("\n[WASM faulted - close and reopen terminal to reset]\n");
            }
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
                
            } catch (Throwable e) {
                // Mark as faulted to prevent further use of corrupted WASM state
                faulted = true;
                // Catch ALL errors including OutOfMemoryError, native errors, etc.
                // This prevents WASM execution failures from crashing the game
                CustomWorldMod.LOGGER.error("Error sending input to WASM", e);
                // Display error in terminal so user can see what went wrong
                terminal.write("\nWASM Error: " + e.getMessage() + "\n");
                terminal.write("[Terminal faulted - close and reopen to reset]\n");
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
