package com.example.customworld.wasm;

import io.github.kawamuray.wasmtime.Engine;
import io.github.kawamuray.wasmtime.Extern;
import io.github.kawamuray.wasmtime.Func;
import io.github.kawamuray.wasmtime.FuncType;
import io.github.kawamuray.wasmtime.ImportType;
import io.github.kawamuray.wasmtime.Instance;
import io.github.kawamuray.wasmtime.Linker;
import io.github.kawamuray.wasmtime.Store;
import io.github.kawamuray.wasmtime.Val;
import io.github.kawamuray.wasmtime.WasmFunctions;
import io.github.kawamuray.wasmtime.WasmValType;
import io.github.kawamuray.wasmtime.WasmtimeException;

import org.junit.jupiter.api.Test;

import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Map;

/**
 * Standalone test for WASM module loading and host function integration.
 * Run with: ./gradlew test --tests WasmImportTest
 */
public class WasmImportTest {
    
    @Test
    public void testSimpleModule() {
        System.out.println("=== Simple Module Test (no imports) ===\n");
        
        // Test with the simple.wasm which has no imports
        Path wasmFile = Path.of("wasm-examples/simple/target/wasm32-unknown-unknown/release/simple.wasm");
        
        if (!Files.exists(wasmFile)) {
            System.out.println("add.wasm not found, skipping simple test");
            return;
        }
        
        // IMPORTANT: Must use Engine from Store to avoid "cross-Engine instantiation" error!
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
        
        Path wasmFile = Path.of("wasm-bin/terminal.wasm");
        
        if (!Files.exists(wasmFile)) {
            System.err.println("ERROR: WASM file not found: " + wasmFile.toAbsolutePath());
            System.err.println("Make sure to build the Rust WASM module first:");
            System.err.println("  cd wasm-examples/rust");
            System.err.println("  cargo build --release --target wasm32-unknown-unknown");
            System.err.println("  cp target/wasm32-unknown-unknown/release/terminal.wasm ../../wasm-bin/");
            return;
        }
        
        // IMPORTANT: Must use Engine from Store to avoid "cross-Engine instantiation" error!
        try (Store<Void> store = Store.withoutData()) {
            Engine engine = store.engine();
            io.github.kawamuray.wasmtime.Module module = io.github.kawamuray.wasmtime.Module.fromFile(engine, wasmFile.toString());
            
            // === STEP 1: Inspect Module Imports ===
            System.out.println("=== Module Imports (what WASM expects) ===");
            var imports = module.imports();
            System.out.println("Total imports: " + imports.length);
            System.out.println();
            
            for (ImportType imp : imports) {
                System.out.println("Import: " + imp.module() + "::" + imp.name());
                System.out.println("  Type class: " + imp.type().getClass().getName());
                System.out.println("  Type: " + imp.type());
                
                // Try to get function type details if it's a function
                var externType = imp.type();
                System.out.println("  Extern type methods:");
                for (var method : externType.getClass().getMethods()) {
                    if (method.getParameterCount() == 0 && !method.getName().equals("getClass") 
                            && !method.getName().equals("hashCode") && !method.getName().equals("toString")
                            && !method.getName().equals("notify") && !method.getName().equals("notifyAll")
                            && !method.getName().equals("wait")) {
                        try {
                            Object result = method.invoke(externType);
                            System.out.println("    " + method.getName() + "() = " + result);
                        } catch (Exception e) {
                            // ignore
                        }
                    }
                }
                System.out.println();
            }
            
            // === STEP 2: Create Host Functions ===
            System.out.println("=== Creating Host Functions ===");
            
            List<Func> hostFuncs = new ArrayList<>();
            Map<String, Extern> funcMap = new HashMap<>();
            
            // terminal_write(ptr: i32, len: i32) -> i32
            System.out.println("Creating terminal_write (i32, i32) -> i32");
            Func writeFunc = WasmFunctions.wrap(store, WasmValType.I32, WasmValType.I32, WasmValType.I32,
                    (Integer ptr, Integer len) -> {
                        System.out.println("  [WASM called terminal_write(" + ptr + ", " + len + ")]");
                        return len;
                    });
            hostFuncs.add(writeFunc);
            funcMap.put("terminal_write", Extern.fromFunc(writeFunc));
            
            // terminal_clear() -> void
            // Try different approaches to create void function
            System.out.println("Creating terminal_clear () -> void");
            
            // Approach 1: Try WasmFunctions.wrap with Runnable (returns I32 0)
            System.out.println("  Approach 1: WasmFunctions.wrap returning dummy I32...");
            try {
                // Since WASM might expect a return value, try returning 0
                Func clearFunc1 = WasmFunctions.wrap(store, WasmValType.I32, () -> {
                    System.out.println("    [WASM called terminal_clear()]");
                    return 0;
                });
                hostFuncs.add(clearFunc1);
                funcMap.put("terminal_clear_v1", Extern.fromFunc(clearFunc1));
                System.out.println("  Approach 1 created successfully");
            } catch (Exception e) {
                System.out.println("  Approach 1 FAILED: " + e.getMessage());
            }
            
            // Approach 2: WasmFunctions.wrap with no return (original approach for reference)
            System.out.println("  Approach 2: WasmFunctions.wrap with Runnable...");
            try {
                Func clearFunc2 = WasmFunctions.wrap(store, () -> {
                    System.out.println("    [WASM called terminal_clear()]");
                });
                hostFuncs.add(clearFunc2);
                funcMap.put("terminal_clear_v2", Extern.fromFunc(clearFunc2));
                System.out.println("  Approach 2 created successfully");
            } catch (Exception e) {
                System.out.println("  Approach 2 FAILED: " + e.getMessage());
            }
            
            // Approach 3: FuncType with Val.Type
            System.out.println("  Approach 3: FuncType with Val.Type...");
            try {
                Func clearFunc3 = new Func(store, new FuncType(new Val.Type[]{}, new Val.Type[]{}), 
                        (caller, params, results) -> {
                            System.out.println("    [WASM called terminal_clear()]");
                        });
                hostFuncs.add(clearFunc3);
                funcMap.put("terminal_clear_v3", Extern.fromFunc(clearFunc3));
                System.out.println("  Approach 3 created successfully");
            } catch (Exception e) {
                System.out.println("  Approach 3 FAILED: " + e.getMessage());
            }
            
            // Approach 4: Use the same Val.Type approach for the main funcMap
            System.out.println("  Using Approach 3 for main funcMap...");
            funcMap.put("terminal_clear", funcMap.get("terminal_clear_v3"));
            
            // Let's also try all approaches during instantiation
            System.out.println("  Will test instantiation with each approach...");
            
            // terminal_set_cursor(x: i32, y: i32) -> void
            System.out.println("Creating terminal_set_cursor (i32, i32) -> void");
            try {
                Func setCursorFunc = new Func(store, 
                        new FuncType(new Val.Type[]{Val.Type.I32, Val.Type.I32}, new Val.Type[]{}), 
                        (caller, params, results) -> {
                            System.out.println("  [WASM called terminal_set_cursor(" + 
                                    params[0].i32() + ", " + params[1].i32() + ")]");
                        });
                hostFuncs.add(setCursorFunc);
                funcMap.put("terminal_set_cursor", Extern.fromFunc(setCursorFunc));
                System.out.println("  Created successfully");
            } catch (Exception e) {
                System.out.println("  FAILED: " + e.getMessage());
            }
            
            // terminal_get_width() -> i32
            System.out.println("Creating terminal_get_width () -> i32");
            
            // Try with WasmFunctions.wrap
            Func getWidthFunc = WasmFunctions.wrap(store, WasmValType.I32, () -> 80);
            hostFuncs.add(getWidthFunc);
            funcMap.put("terminal_get_width_wrapped", Extern.fromFunc(getWidthFunc));
            
            // Also try with FuncType constructor
            try {
                Func getWidthFunc2 = new Func(store, new FuncType(new Val.Type[]{}, new Val.Type[]{Val.Type.I32}), 
                        (caller, params, results) -> {
                            results[0] = Val.fromI32(80);
                        });
                hostFuncs.add(getWidthFunc2);
                funcMap.put("terminal_get_width", Extern.fromFunc(getWidthFunc2));
                System.out.println("  Created with FuncType");
            } catch (Exception e) {
                System.out.println("  FuncType approach failed: " + e.getMessage());
                // Fall back to wrapped version
                funcMap.put("terminal_get_width", funcMap.get("terminal_get_width_wrapped"));
            }
            
            // terminal_get_height() -> i32
            System.out.println("Creating terminal_get_height () -> i32");
            try {
                Func getHeightFunc = new Func(store, new FuncType(new Val.Type[]{}, new Val.Type[]{Val.Type.I32}), 
                        (caller, params, results) -> {
                            results[0] = Val.fromI32(24);
                        });
                hostFuncs.add(getHeightFunc);
                funcMap.put("terminal_get_height", Extern.fromFunc(getHeightFunc));
                System.out.println("  Created with FuncType");
            } catch (Exception e) {
                System.out.println("  FuncType approach failed: " + e.getMessage());
                Func getHeightFunc2 = WasmFunctions.wrap(store, WasmValType.I32, () -> 24);
                hostFuncs.add(getHeightFunc2);
                funcMap.put("terminal_get_height", Extern.fromFunc(getHeightFunc2));
            }
            
            System.out.println();
            
            // === STEP 3: Build Import List in Correct Order ===
            System.out.println("=== Building Import List ===");
            List<Extern> orderedImports = new ArrayList<>();
            
            for (ImportType imp : imports) {
                String name = imp.name();
                System.out.print("Looking up: " + name + " ... ");
                
                if (funcMap.containsKey(name)) {
                    orderedImports.add(funcMap.get(name));
                    System.out.println("FOUND");
                } else {
                    System.out.println("NOT FOUND - this will cause instantiation to fail!");
                }
            }
            
            System.out.println();
            System.out.println("Providing " + orderedImports.size() + " imports for " + imports.length + " expected");
            System.out.println();
            
            // === STEP 4: Try to Instantiate with different approaches ===
            System.out.println("=== Attempting Instantiation ===");
            
            // Try with v1 (returning I32)
            System.out.println("\nTrying with Approach 1 (terminal_clear returns I32)...");
            if (funcMap.containsKey("terminal_clear_v1")) {
                List<Extern> testImports1 = new ArrayList<>();
                for (ImportType imp : imports) {
                    String name = imp.name();
                    if (name.equals("terminal_clear")) {
                        testImports1.add(funcMap.get("terminal_clear_v1"));
                    } else if (funcMap.containsKey(name)) {
                        testImports1.add(funcMap.get(name));
                    }
                }
                try {
                    Instance instance = new Instance(store, module, testImports1);
                    System.out.println("  SUCCESS with Approach 1!");
                    instance.close();
                } catch (WasmtimeException e) {
                    System.out.println("  FAILED: " + e.getMessage());
                }
            }
            
            // Try with v2 (WasmFunctions.wrap Runnable)
            System.out.println("\nTrying with Approach 2 (WasmFunctions.wrap Runnable)...");
            if (funcMap.containsKey("terminal_clear_v2")) {
                List<Extern> testImports2 = new ArrayList<>();
                for (ImportType imp : imports) {
                    String name = imp.name();
                    if (name.equals("terminal_clear")) {
                        testImports2.add(funcMap.get("terminal_clear_v2"));
                    } else if (funcMap.containsKey(name)) {
                        testImports2.add(funcMap.get(name));
                    }
                }
                try {
                    Instance instance = new Instance(store, module, testImports2);
                    System.out.println("  SUCCESS with Approach 2!");
                    instance.close();
                } catch (WasmtimeException e) {
                    System.out.println("  FAILED: " + e.getMessage());
                }
            }
            
            // Try with v3 (FuncType Val.Type)
            System.out.println("\nTrying with Approach 3 (FuncType Val.Type)...");
            if (funcMap.containsKey("terminal_clear_v3")) {
                List<Extern> testImports3 = new ArrayList<>();
                for (ImportType imp : imports) {
                    String name = imp.name();
                    if (name.equals("terminal_clear")) {
                        testImports3.add(funcMap.get("terminal_clear_v3"));
                    } else if (funcMap.containsKey(name)) {
                        testImports3.add(funcMap.get(name));
                    }
                }
                try {
                    Instance instance = new Instance(store, module, testImports3);
                    System.out.println("  SUCCESS with Approach 3!");
                    instance.close();
                } catch (WasmtimeException e) {
                    System.out.println("  FAILED: " + e.getMessage());
                }
            }
            
            // Try with Linker approach
            System.out.println("\nTrying with Linker approach...");
            try {
                Linker linker = new Linker(engine);
                
                // Define all functions with linker
                linker.define(store, "env", "terminal_write", funcMap.get("terminal_write"));
                linker.define(store, "env", "terminal_clear", funcMap.get("terminal_clear"));
                linker.define(store, "env", "terminal_get_width", funcMap.get("terminal_get_width"));
                linker.define(store, "env", "terminal_get_height", funcMap.get("terminal_get_height"));
                
                // Try to instantiate via linker
                // Note: Linker.instantiate may not exist in this version
                System.out.println("  Linker definitions added, but instantiate() may not exist in v0.19.0");
                linker.close();
            } catch (Exception e) {
                System.out.println("  Linker approach failed: " + e.getMessage());
            }
            
            // Final try with orderedImports
            System.out.println("\nFinal try with orderedImports...");
            try {
                Instance instance = new Instance(store, module, orderedImports);
                System.out.println("  SUCCESS!");
                
                // Try to get memory
                var memoryOpt = instance.getMemory(store, "memory");
                if (memoryOpt.isPresent()) {
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
            } catch (WasmtimeException e) {
                System.out.println("  FAILED: " + e.getMessage());
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
}
