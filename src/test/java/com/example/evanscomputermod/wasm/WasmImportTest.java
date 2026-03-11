package com.example.evanscomputermod.wasm;

import io.github.kawamuray.wasmtime.Engine;
import io.github.kawamuray.wasmtime.Extern;
import io.github.kawamuray.wasmtime.Func;
import io.github.kawamuray.wasmtime.FuncType;
import io.github.kawamuray.wasmtime.ImportType;
import io.github.kawamuray.wasmtime.Instance;
import io.github.kawamuray.wasmtime.Memory;
import io.github.kawamuray.wasmtime.Store;
import io.github.kawamuray.wasmtime.Val;
import io.github.kawamuray.wasmtime.WasmFunctions;
import io.github.kawamuray.wasmtime.WasmValType;

import org.junit.jupiter.api.Test;

import java.nio.ByteBuffer;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import java.util.Random;

/**
 * Standalone test for WASM module loading and host function integration.
 * Run with: ./gradlew test --tests WasmImportTest
 */
public class WasmImportTest {
    
    private Store<Void> store;
    private List<Func> hostFuncs = new ArrayList<>();
    private Map<String, Extern> funcMap = new HashMap<>();
    private Memory memory;
    
    @Test
    public void testSimpleModule() {
        System.out.println("=== Simple Module Test (no imports) ===\n");
        
        Path wasmFile = Path.of("wasm-examples/simple/target/wasm32-unknown-unknown/release/simple.wasm");
        
        if (!Files.exists(wasmFile)) {
            System.out.println("simple.wasm not found, skipping simple test");
            return;
        }
        
        try (Store<Void> store = Store.withoutData()) {
            Engine engine = store.engine();
            io.github.kawamuray.wasmtime.Module module = io.github.kawamuray.wasmtime.Module.fromFile(engine, wasmFile.toString());
            
            System.out.println("Module loaded: " + wasmFile);
            System.out.println("Imports: " + module.imports().length);
            
            Instance instance = new Instance(store, module, List.of());
            System.out.println("Instance created successfully!");
            
            var addFunc = instance.getFunc(store, "add");
            if (addFunc.isPresent()) {
                System.out.println("Found 'add' function");
                var results = addFunc.get().call(store, Val.fromI32(2), Val.fromI32(3));
                System.out.println("add(2, 3) = " + results[0].i32());
            }
            
            instance.close();
        } catch (Exception e) {
            System.err.println("Simple module test FAILED: " + e.getMessage());
            e.printStackTrace(System.err);
        }
    }
    
    @Test
    public void testWasmImports() {
        System.out.println("=== WASM Import Test ===\n");
        
        Path wasmFile = Path.of("wasm-bin/terminal_os.wasm");
        
        if (!Files.exists(wasmFile)) {
            System.err.println("ERROR: WASM file not found: " + wasmFile.toAbsolutePath());
            System.err.println("Make sure to build the Rust WASM module first:");
            System.err.println("  cd operating-system/rust");
            System.err.println("  cargo build --release --target wasm32-unknown-unknown");
            System.err.println("  cp target/wasm32-unknown-unknown/release/terminal_os.wasm ../../wasm-bin/");
            return;
        }
        
        try (Store<Void> testStore = Store.withoutData()) {
            this.store = testStore;
            Engine engine = store.engine();
            io.github.kawamuray.wasmtime.Module module = io.github.kawamuray.wasmtime.Module.fromFile(engine, wasmFile.toString());
            
            // === STEP 1: Inspect Module Imports ===
            System.out.println("=== Module Imports ===");
            var imports = module.imports();
            System.out.println("Total imports: " + imports.length);
            System.out.println();
            
            // Print detailed type info for wasm-bindgen imports
            for (ImportType imp : imports) {
                if (imp.module().startsWith("__wbindgen")) {
                    System.out.println("Import: " + imp.module() + "::" + imp.name());
                    System.out.println("  Type: " + imp.type());
                }
            }
            System.out.println();
            
            // === STEP 2: Create All Host Functions ===
            System.out.println("=== Creating Host Functions ===");
            createAllHostFunctions();
            System.out.println("Created " + funcMap.size() + " host functions");
            System.out.println();
            
            // === STEP 3: Build Import List ===
            System.out.println("=== Building Import List ===");
            List<Extern> orderedImports = new ArrayList<>();
            int found = 0;
            int notFound = 0;
            
            for (ImportType imp : imports) {
                String name = imp.name();
                
                if (funcMap.containsKey(name)) {
                    orderedImports.add(funcMap.get(name));
                    found++;
                } else {
                    System.out.println("NOT FOUND: " + imp.module() + "::" + name);
                    notFound++;
                }
            }
            
            System.out.println();
            System.out.println("Found: " + found + ", Not found: " + notFound);
            System.out.println("Providing " + orderedImports.size() + " imports for " + imports.length + " expected");
            System.out.println();
            
            if (notFound > 0) {
                System.out.println("WARNING: Some imports are missing! Instantiation will fail.");
            }
            
            // === STEP 4: Instantiate ===
            if (orderedImports.size() == imports.length) {
                System.out.println("=== Attempting Instantiation ===");
                try {
                    Instance instance = new Instance(store, module, orderedImports);
                    System.out.println("SUCCESS! Instance created.");
                    
                    // Get memory
                    var memoryOpt = instance.getMemory(store, "memory");
                    if (memoryOpt.isPresent()) {
                        this.memory = memoryOpt.get();
                        System.out.println("Memory export found.");
                    }
                    
                    // Try to call main
                    var mainFunc = instance.getFunc(store, "main");
                    if (mainFunc.isPresent()) {
                        System.out.println("Found 'main' function, calling...");
                        mainFunc.get().call(store);
                        System.out.println("main() executed successfully!");
                    }
                    
                    instance.close();
                } catch (Exception e) {
                    System.err.println("Instantiation FAILED: " + e.getMessage());
                    e.printStackTrace();
                }
            }
            
            // Cleanup
            for (Func func : hostFuncs) {
                func.close();
            }
            
        } catch (Exception e) {
            System.err.println("ERROR: " + e.getMessage());
            e.printStackTrace();
        }
        
        System.out.println("\n=== Test Complete ===");
    }
    
    /**
     * Test that specifically tests the Python REPL initialization.
     * This simulates typing "python" in the terminal.
     * Run with: ./gradlew test --tests WasmImportTest.testPythonRepl
     */
    @Test
    public void testPythonRepl() {
        System.out.println("=== Python REPL Test ===\n");
        
        Path wasmFile = Path.of("wasm-bin/terminal_os.wasm");
        
        if (!Files.exists(wasmFile)) {
            System.err.println("ERROR: WASM file not found: " + wasmFile.toAbsolutePath());
            return;
        }
        
        try (Store<Void> testStore = Store.withoutData()) {
            this.store = testStore;
            this.hostFuncs = new ArrayList<>();
            this.funcMap = new HashMap<>();
            
            Engine engine = store.engine();
            io.github.kawamuray.wasmtime.Module module = io.github.kawamuray.wasmtime.Module.fromFile(engine, wasmFile.toString());
            
            System.out.println("Module loaded: " + wasmFile);
            System.out.println("Creating host functions with logging...");
            
            // Create host functions with logging
            createAllHostFunctionsWithLogging();
            
            System.out.println("Created " + funcMap.size() + " host functions");
            
            // Build import list
            var imports = module.imports();
            List<Extern> orderedImports = new ArrayList<>();
            
            for (ImportType imp : imports) {
                String name = imp.name();
                if (funcMap.containsKey(name)) {
                    orderedImports.add(funcMap.get(name));
                } else {
                    System.err.println("MISSING IMPORT: " + name);
                }
            }
            
            if (orderedImports.size() != imports.length) {
                System.err.println("Import count mismatch! Expected " + imports.length + ", got " + orderedImports.size());
                return;
            }
            
            // Create instance
            System.out.println("\n=== Creating Instance ===");
            Instance instance = new Instance(store, module, orderedImports);
            System.out.println("Instance created successfully!");
            
            // Get memory
            var memoryOpt = instance.getMemory(store, "memory");
            if (memoryOpt.isPresent()) {
                this.memory = memoryOpt.get();
                System.out.println("Memory export found.");
            } else {
                System.err.println("Memory export NOT found!");
                return;
            }
            
            // Call main() to initialize OS
            System.out.println("\n=== Calling main() ===");
            var mainFunc = instance.getFunc(store, "main");
            if (mainFunc.isPresent()) {
                mainFunc.get().call(store);
                System.out.println("main() executed - OS initialized");
            } else {
                System.err.println("main() function not found!");
                return;
            }
            
            // Find on_input function
            var onInputFunc = instance.getFunc(store, "on_input");
            if (onInputFunc.isEmpty()) {
                System.err.println("on_input function not found!");
                return;
            }
            System.out.println("on_input function found.");
            
            // Send "python" command character by character, then Enter
            System.out.println("\n=== Sending 'python' command ===");
            String command = "python\n";
            
            for (char c : command.toCharArray()) {
                byte[] bytes = String.valueOf(c).getBytes(java.nio.charset.StandardCharsets.UTF_8);
                ByteBuffer buffer = memory.buffer(store);
                buffer.position(0x10000);
                buffer.put(bytes);
                
                try {
                    onInputFunc.get().call(store, Val.fromI32(0x10000), Val.fromI32(bytes.length));
                } catch (Exception e) {
                    System.err.println("\nERROR at char '" + c + "': " + e.getMessage());
                    e.printStackTrace();
                    break;
                }
            }
            
            System.out.println("\n=== Python REPL Test Complete ===");
            
            instance.close();
            
        } catch (Exception e) {
            System.err.println("TEST FAILED: " + e.getMessage());
            e.printStackTrace();
        }
        
        // Cleanup
        for (Func func : hostFuncs) {
            func.close();
        }
    }
    
    /**
     * Creates all host functions needed by terminal_os.wasm
     */
    private void createAllHostFunctions() {
        // === Terminal functions ===
        addFunc("terminal_write", new Val.Type[]{Val.Type.I32, Val.Type.I32}, new Val.Type[]{Val.Type.I32},
                (caller, params, results) -> {
                    results[0] = Val.fromI32(params[1].i32()); // Return length
                });
        
        addFuncVoid("terminal_clear");
        
        addFuncVoid("terminal_set_cursor", Val.Type.I32, Val.Type.I32);
        
        addFuncI32Return("terminal_get_width", 80);
        addFuncI32Return("terminal_get_height", 24);
        
        // === File system functions ===
        addFunc("file_write", new Val.Type[]{Val.Type.I32, Val.Type.I32, Val.Type.I32, Val.Type.I32}, 
                new Val.Type[]{Val.Type.I32}, (caller, params, results) -> results[0] = Val.fromI32(0));
        
        addFunc("file_read", new Val.Type[]{Val.Type.I32, Val.Type.I32, Val.Type.I32, Val.Type.I32}, 
                new Val.Type[]{Val.Type.I32}, (caller, params, results) -> results[0] = Val.fromI32(-1));
        
        addFunc("file_size", new Val.Type[]{Val.Type.I32, Val.Type.I32}, 
                new Val.Type[]{Val.Type.I32}, (caller, params, results) -> results[0] = Val.fromI32(-1));
        
        addFunc("file_exists", new Val.Type[]{Val.Type.I32, Val.Type.I32}, 
                new Val.Type[]{Val.Type.I32}, (caller, params, results) -> results[0] = Val.fromI32(0));
        
        addFunc("file_delete", new Val.Type[]{Val.Type.I32, Val.Type.I32}, 
                new Val.Type[]{Val.Type.I32}, (caller, params, results) -> results[0] = Val.fromI32(0));
        
        addFunc("file_list", new Val.Type[]{Val.Type.I32, Val.Type.I32}, 
                new Val.Type[]{Val.Type.I32}, (caller, params, results) -> results[0] = Val.fromI32(0));
        
        // === Custom getrandom ===
        addFunc("__getrandom_v03_custom", new Val.Type[]{Val.Type.I32, Val.Type.I32}, 
                new Val.Type[]{Val.Type.I32}, (caller, params, results) -> {
                    // Fill memory with random bytes
                    if (memory != null) {
                        int ptr = params[0].i32();
                        int len = params[1].i32();
                        ByteBuffer buf = memory.buffer(store);
                        buf.position(ptr);
                        byte[] bytes = new byte[len];
                        new Random().nextBytes(bytes);
                        buf.put(bytes);
                    }
                    results[0] = Val.fromI32(0);  // Success
                });
        
        // === wasm-bindgen stubs ===
        // These are required by RustPython's dependencies
        
        addFuncVoid("__wbindgen_describe", Val.Type.I32);  // type 1
        addFuncI32Return("__wbindgen_describe_cast", 0, Val.Type.I32, Val.Type.I32);  // type 2
        addFuncVoid("__wbindgen_object_drop_ref", Val.Type.I32);  // type 1
        addFuncI32Return("__wbindgen_object_clone_ref", 0, Val.Type.I32);  // type 4
        
        // Boolean checks (return false/0 for most, true/1 for is_undefined)
        addFuncI32Return("__wbg___wbindgen_is_object_ce774f3490692386", 0, Val.Type.I32);
        addFuncI32Return("__wbg___wbindgen_is_string_704ef9c8fc131030", 0, Val.Type.I32);
        addFuncI32Return("__wbg___wbindgen_is_function_8d400b8b1af978cd", 0, Val.Type.I32);
        addFuncI32Return("__wbg___wbindgen_is_undefined_f6b95eab589e0269", 1, Val.Type.I32);
        
        // Date/time stubs - some return f64!
        addFuncI32Return("__wbg_new_b2db8aa2650f793a", 0, Val.Type.I32);  // type 4: (i32) -> i32
        addFuncF64Return("__wbg_getTimezoneOffset_45389e26d6f46823", 0.0, Val.Type.I32);  // type 20: (i32) -> f64
        addFuncI32Return("__wbg_new_0_23cedd11d9b40c9d", 0);  // type 7: () -> i32
        addFuncF64Return("__wbg_getTime_ad1e9878a735af08", 0.0, Val.Type.I32);  // type 20: (i32) -> f64
        addFuncF64Return("__wbg_now_2c70f2474e348581", (double) System.currentTimeMillis());  // type 21: () -> f64
        
        // Crypto/random stubs
        addFuncI32Return("__wbg_crypto_574e78ad8b13b65f", 0, Val.Type.I32);  // type 4: (i32) -> i32
        addFuncI32Return("__wbg_msCrypto_a61aeb35a24c1329", 0, Val.Type.I32);  // type 4: (i32) -> i32
        addFuncVoid("__wbg_randomFillSync_ac0988aba3254290", Val.Type.I32, Val.Type.I32);  // type 16: (i32, i32) -> void
        addFuncVoid("__wbg_getRandomValues_b8f5dbd5f3995a9e", Val.Type.I32, Val.Type.I32);  // type 16: (i32, i32) -> void
        
        // Node.js stubs  
        addFuncI32Return("__wbg_process_dc0fbacc7c1c06f7", 0, Val.Type.I32);  // type 4: (i32) -> i32
        addFuncI32Return("__wbg_versions_c01dfd4722a88165", 0, Val.Type.I32);  // type 4: (i32) -> i32
        addFuncI32Return("__wbg_node_905d3e251edff8a2", 0, Val.Type.I32);  // type 4: (i32) -> i32
        addFuncI32Return("__wbg_require_60cc747a6bc5215a", 0);  // type 7: () -> i32
        
        // Function call stubs
        addFuncI32Return("__wbg_call_3020136f7a2d6e44", 0, Val.Type.I32, Val.Type.I32, Val.Type.I32);  // type 3
        addFuncI32Return("__wbg_call_abb4ff46ce38be40", 0, Val.Type.I32, Val.Type.I32);  // type 2
        
        // Global/window stubs
        addFuncI32Return("__wbg_static_accessor_GLOBAL_769e6b65d6557335", 0);
        addFuncI32Return("__wbg_static_accessor_GLOBAL_THIS_60cf02db4de8e1c1", 0);
        addFuncI32Return("__wbg_static_accessor_WINDOW_a8924b26aa92d024", 0);
        addFuncI32Return("__wbg_static_accessor_SELF_08f5a74c69739274", 0);
        
        // Array stubs
        addFuncI32Return("__wbg_new_with_length_aa5eaf41d35235e5", 0, Val.Type.I32);
        addFuncI32Return("__wbg_subarray_845f2f5bce7d061a", 0, Val.Type.I32, Val.Type.I32, Val.Type.I32);
        addFuncI32Return("__wbg_length_22ac23eaec9d8053", 0, Val.Type.I32);
        
        // Misc stubs
        addFuncI32Return("__wbg_new_no_args_cb138f77cf6151ee", 0, Val.Type.I32, Val.Type.I32);  // type 2
        addFuncVoid("__wbg_prototypesetcall_dfe9b766cdc1f1fd", Val.Type.I32, Val.Type.I32, Val.Type.I32);  // type 0
        addFuncVoid("__wbg_error_d01e9edc65d6e61f", Val.Type.I32, Val.Type.I32);  // type 16
        addFuncVoid("__wbg___wbindgen_throw_dd24417ed36fc46e", Val.Type.I32, Val.Type.I32);  // type 16
        
        // Externref table stubs
        addFuncVoid("__wbindgen_externref_table_set_null", Val.Type.I32);
        addFuncI32Return("__wbindgen_externref_table_grow", 0, Val.Type.I32);
    }
    
    // Counter for object handles
    private java.util.concurrent.atomic.AtomicInteger nextHandle = new java.util.concurrent.atomic.AtomicInteger(1);
    
    /**
     * Creates host functions with logging for debugging Python REPL.
     */
    private void createAllHostFunctionsWithLogging() {
        // === Terminal functions - print actual output ===
        addFunc("terminal_write", new Val.Type[]{Val.Type.I32, Val.Type.I32}, new Val.Type[]{Val.Type.I32},
                (caller, params, results) -> {
                    int ptr = params[0].i32();
                    int len = params[1].i32();
                    if (memory != null && len > 0 && len < 10000) {
                        ByteBuffer buf = memory.buffer(store);
                        byte[] bytes = new byte[len];
                        buf.position(ptr);
                        buf.get(bytes);
                        String text = new String(bytes, java.nio.charset.StandardCharsets.UTF_8);
                        System.out.print(text);  // Print terminal output
                    }
                    results[0] = Val.fromI32(len);
                });
        
        addFuncVoid("terminal_clear");
        addFuncVoid("terminal_set_cursor", Val.Type.I32, Val.Type.I32);
        addFuncI32Return("terminal_get_width", 80);
        addFuncI32Return("terminal_get_height", 24);
        
        // === File system functions ===
        addFunc("file_write", new Val.Type[]{Val.Type.I32, Val.Type.I32, Val.Type.I32, Val.Type.I32}, 
                new Val.Type[]{Val.Type.I32}, (caller, params, results) -> results[0] = Val.fromI32(0));
        addFunc("file_read", new Val.Type[]{Val.Type.I32, Val.Type.I32, Val.Type.I32, Val.Type.I32}, 
                new Val.Type[]{Val.Type.I32}, (caller, params, results) -> results[0] = Val.fromI32(-1));
        addFunc("file_size", new Val.Type[]{Val.Type.I32, Val.Type.I32}, 
                new Val.Type[]{Val.Type.I32}, (caller, params, results) -> results[0] = Val.fromI32(-1));
        addFunc("file_exists", new Val.Type[]{Val.Type.I32, Val.Type.I32}, 
                new Val.Type[]{Val.Type.I32}, (caller, params, results) -> results[0] = Val.fromI32(0));
        addFunc("file_delete", new Val.Type[]{Val.Type.I32, Val.Type.I32}, 
                new Val.Type[]{Val.Type.I32}, (caller, params, results) -> results[0] = Val.fromI32(0));
        addFunc("file_list", new Val.Type[]{Val.Type.I32, Val.Type.I32}, 
                new Val.Type[]{Val.Type.I32}, (caller, params, results) -> results[0] = Val.fromI32(0));
        
        // === Custom getrandom - fill with random bytes ===
        addFunc("__getrandom_v03_custom", new Val.Type[]{Val.Type.I32, Val.Type.I32}, 
                new Val.Type[]{Val.Type.I32}, (caller, params, results) -> {
                    if (memory != null) {
                        int ptr = params[0].i32();
                        int len = params[1].i32();
                        ByteBuffer buf = memory.buffer(store);
                        buf.position(ptr);
                        byte[] bytes = new byte[len];
                        new Random().nextBytes(bytes);
                        buf.put(bytes);
                    }
                    results[0] = Val.fromI32(0);
                });
        
        // === wasm-bindgen core functions with logging ===
        addFuncVoidLogged("__wbindgen_describe", Val.Type.I32);
        addFuncI32Logged("__wbindgen_describe_cast", 0, Val.Type.I32, Val.Type.I32);
        addFuncVoidLogged("__wbindgen_object_drop_ref", Val.Type.I32);
        
        // Object clone ref - return incrementing handles
        addFunc("__wbindgen_object_clone_ref", new Val.Type[]{Val.Type.I32}, new Val.Type[]{Val.Type.I32},
                (caller, params, results) -> {
                    int handle = nextHandle.getAndIncrement();
                    System.out.println("[STUB] object_clone_ref(" + params[0].i32() + ") -> " + handle);
                    results[0] = Val.fromI32(handle);
                });
        
        // Boolean checks - is_object returns true (1) for non-zero handles
        addFunc("__wbg___wbindgen_is_object_ce774f3490692386", new Val.Type[]{Val.Type.I32}, new Val.Type[]{Val.Type.I32},
                (caller, params, results) -> {
                    int handle = params[0].i32();
                    int result = (handle != 0) ? 1 : 0;  // Non-zero handles are valid objects
                    System.out.println("[STUB] is_object(" + handle + ") -> " + result);
                    results[0] = Val.fromI32(result);
                });
        addFuncI32Logged("__wbg___wbindgen_is_string_704ef9c8fc131030", 0, Val.Type.I32);
        addFuncI32Logged("__wbg___wbindgen_is_function_8d400b8b1af978cd", 0, Val.Type.I32);
        addFuncI32Logged("__wbg___wbindgen_is_undefined_f6b95eab589e0269", 1, Val.Type.I32);
        
        // Date/time functions - return real values
        addFunc("__wbg_new_b2db8aa2650f793a", new Val.Type[]{Val.Type.I32}, new Val.Type[]{Val.Type.I32},
                (caller, params, results) -> {
                    int handle = nextHandle.getAndIncrement();
                    System.out.println("[STUB] Date.new(timestamp) -> handle " + handle);
                    results[0] = Val.fromI32(handle);
                });
        
        addFunc("__wbg_getTimezoneOffset_45389e26d6f46823", new Val.Type[]{Val.Type.I32}, new Val.Type[]{Val.Type.F64},
                (caller, params, results) -> {
                    double offset = -java.util.TimeZone.getDefault().getRawOffset() / 60000.0;
                    System.out.println("[STUB] Date.getTimezoneOffset() -> " + offset);
                    results[0] = Val.fromF64(offset);
                });
        
        addFunc("__wbg_new_0_23cedd11d9b40c9d", new Val.Type[]{}, new Val.Type[]{Val.Type.I32},
                (caller, params, results) -> {
                    int handle = nextHandle.getAndIncrement();
                    System.out.println("[STUB] Date.new() -> handle " + handle);
                    results[0] = Val.fromI32(handle);
                });
        
        addFunc("__wbg_getTime_ad1e9878a735af08", new Val.Type[]{Val.Type.I32}, new Val.Type[]{Val.Type.F64},
                (caller, params, results) -> {
                    double time = System.currentTimeMillis();
                    System.out.println("[STUB] Date.getTime() -> " + time);
                    results[0] = Val.fromF64(time);
                });
        
        addFunc("__wbg_now_2c70f2474e348581", new Val.Type[]{}, new Val.Type[]{Val.Type.F64},
                (caller, params, results) -> {
                    double time = System.currentTimeMillis();
                    System.out.println("[STUB] Date.now() -> " + time);
                    results[0] = Val.fromF64(time);
                });
        
        // Crypto - return valid handle so getrandom knows crypto is available
        addFunc("__wbg_crypto_574e78ad8b13b65f", new Val.Type[]{Val.Type.I32}, new Val.Type[]{Val.Type.I32},
                (caller, params, results) -> {
                    int handle = nextHandle.getAndIncrement();
                    System.out.println("[STUB] crypto object -> handle " + handle);
                    results[0] = Val.fromI32(handle);
                });
        
        addFuncI32Logged("__wbg_msCrypto_a61aeb35a24c1329", 0, Val.Type.I32);
        addFuncVoidLogged("__wbg_randomFillSync_ac0988aba3254290", Val.Type.I32, Val.Type.I32);
        addFuncVoidLogged("__wbg_getRandomValues_b8f5dbd5f3995a9e", Val.Type.I32, Val.Type.I32);
        
        // Node.js stubs
        addFuncI32Logged("__wbg_process_dc0fbacc7c1c06f7", 0, Val.Type.I32);
        addFuncI32Logged("__wbg_versions_c01dfd4722a88165", 0, Val.Type.I32);
        addFuncI32Logged("__wbg_node_905d3e251edff8a2", 0, Val.Type.I32);
        addFuncI32Logged("__wbg_require_60cc747a6bc5215a", 0);
        
        // Function call stubs
        addFuncI32Logged("__wbg_call_3020136f7a2d6e44", 0, Val.Type.I32, Val.Type.I32, Val.Type.I32);
        addFuncI32Logged("__wbg_call_abb4ff46ce38be40", 0, Val.Type.I32, Val.Type.I32);
        
        // Global/window stubs
        addFuncI32Logged("__wbg_static_accessor_GLOBAL_769e6b65d6557335", 0);
        addFuncI32Logged("__wbg_static_accessor_GLOBAL_THIS_60cf02db4de8e1c1", 0);
        addFuncI32Logged("__wbg_static_accessor_WINDOW_a8924b26aa92d024", 0);
        addFuncI32Logged("__wbg_static_accessor_SELF_08f5a74c69739274", 0);
        
        // Array stubs - return valid handles
        addFunc("__wbg_new_with_length_aa5eaf41d35235e5", new Val.Type[]{Val.Type.I32}, new Val.Type[]{Val.Type.I32},
                (caller, params, results) -> {
                    int handle = nextHandle.getAndIncrement();
                    System.out.println("[STUB] new Uint8Array(" + params[0].i32() + ") -> handle " + handle);
                    results[0] = Val.fromI32(handle);
                });
        
        addFunc("__wbg_subarray_845f2f5bce7d061a", new Val.Type[]{Val.Type.I32, Val.Type.I32, Val.Type.I32}, new Val.Type[]{Val.Type.I32},
                (caller, params, results) -> {
                    int handle = nextHandle.getAndIncrement();
                    System.out.println("[STUB] Uint8Array.subarray() -> handle " + handle);
                    results[0] = Val.fromI32(handle);
                });
        
        addFuncI32Logged("__wbg_length_22ac23eaec9d8053", 0, Val.Type.I32);
        
        // Misc stubs
        addFuncI32Logged("__wbg_new_no_args_cb138f77cf6151ee", 0, Val.Type.I32, Val.Type.I32);
        addFuncVoidLogged("__wbg_prototypesetcall_dfe9b766cdc1f1fd", Val.Type.I32, Val.Type.I32, Val.Type.I32);
        
        // Error handling - read error message from memory and print it
        addFunc("__wbg_error_d01e9edc65d6e61f", new Val.Type[]{Val.Type.I32, Val.Type.I32}, new Val.Type[]{},
                (caller, params, results) -> {
                    int ptr = params[0].i32();
                    int len = params[1].i32();
                    String msg = readStringFromMemory(ptr, len);
                    System.err.println("[WASM ERROR] " + msg);
                });
        
        addFunc("__wbg___wbindgen_throw_dd24417ed36fc46e", new Val.Type[]{Val.Type.I32, Val.Type.I32}, new Val.Type[]{},
                (caller, params, results) -> {
                    int ptr = params[0].i32();
                    int len = params[1].i32();
                    String msg = readStringFromMemory(ptr, len);
                    System.err.println("[WASM THROW] " + msg);
                    throw new RuntimeException("WASM throw: " + msg);
                });
        
        // Externref table stubs
        addFuncVoidLogged("__wbindgen_externref_table_set_null", Val.Type.I32);
        
        addFunc("__wbindgen_externref_table_grow", new Val.Type[]{Val.Type.I32}, new Val.Type[]{Val.Type.I32},
                (caller, params, results) -> {
                    int delta = params[0].i32();
                    int oldSize = nextHandle.get();
                    nextHandle.addAndGet(delta);
                    System.out.println("[STUB] externref_table_grow(" + delta + ") -> " + oldSize);
                    results[0] = Val.fromI32(oldSize);
                });
    }
    
    private String readStringFromMemory(int ptr, int len) {
        if (memory == null || len <= 0 || len > 10000) {
            return "(unable to read)";
        }
        try {
            ByteBuffer buf = memory.buffer(store);
            byte[] bytes = new byte[len];
            buf.position(ptr);
            buf.get(bytes);
            return new String(bytes, java.nio.charset.StandardCharsets.UTF_8);
        } catch (Exception e) {
            return "(error reading: " + e.getMessage() + ")";
        }
    }
    
    private void addFunc(String name, Val.Type[] params, Val.Type[] results, 
                         io.github.kawamuray.wasmtime.Func.Handler handler) {
        Func func = new Func(store, new FuncType(params, results), handler);
        hostFuncs.add(func);
        funcMap.put(name, Extern.fromFunc(func));
    }
    
    private void addFuncVoid(String name, Val.Type... params) {
        Func func = new Func(store, new FuncType(params, new Val.Type[]{}), 
                (caller, p, r) -> {});
        hostFuncs.add(func);
        funcMap.put(name, Extern.fromFunc(func));
    }
    
    private void addFuncVoidLogged(String name, Val.Type... params) {
        final String funcName = name;
        Func func = new Func(store, new FuncType(params, new Val.Type[]{}), 
                (caller, p, r) -> {
                    System.out.println("[STUB] " + funcName + " called");
                });
        hostFuncs.add(func);
        funcMap.put(name, Extern.fromFunc(func));
    }
    
    private void addFuncI32Return(String name, int value, Val.Type... params) {
        Func func = new Func(store, new FuncType(params, new Val.Type[]{Val.Type.I32}), 
                (caller, p, r) -> r[0] = Val.fromI32(value));
        hostFuncs.add(func);
        funcMap.put(name, Extern.fromFunc(func));
    }
    
    private void addFuncI32Logged(String name, int value, Val.Type... params) {
        final String funcName = name;
        final int retVal = value;
        Func func = new Func(store, new FuncType(params, new Val.Type[]{Val.Type.I32}), 
                (caller, p, r) -> {
                    System.out.println("[STUB] " + funcName + " -> " + retVal);
                    r[0] = Val.fromI32(retVal);
                });
        hostFuncs.add(func);
        funcMap.put(name, Extern.fromFunc(func));
    }
    
    private void addFuncF64Return(String name, double value, Val.Type... params) {
        Func func = new Func(store, new FuncType(params, new Val.Type[]{Val.Type.F64}), 
                (caller, p, r) -> r[0] = Val.fromF64(value));
        hostFuncs.add(func);
        funcMap.put(name, Extern.fromFunc(func));
    }
}
