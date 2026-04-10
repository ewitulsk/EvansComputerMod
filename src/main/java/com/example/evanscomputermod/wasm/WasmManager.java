package com.example.evanscomputermod.wasm;

import com.example.evanscomputermod.EvansComputerMod;
import io.github.kawamuray.wasmtime.Engine;
import io.github.kawamuray.wasmtime.Extern;
import io.github.kawamuray.wasmtime.Func;
import io.github.kawamuray.wasmtime.Instance;
import io.github.kawamuray.wasmtime.Module;
import io.github.kawamuray.wasmtime.Store;
import io.github.kawamuray.wasmtime.Val;
import io.github.kawamuray.wasmtime.WasmtimeException;

import java.io.IOException;
import java.io.InputStream;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.StandardCopyOption;
import java.util.ArrayList;
import java.util.List;

/**
 * Manages WASM module loading and function execution.
 * Creates and manages the wasm-bin/ directory for WASM binaries.
 */
public class WasmManager {
    
    private static final String WASM_BIN_FOLDER = "wasm-bin";
    /** Core WASM file that must always be extracted (the OS kernel). */
    private static final String CORE_WASM_FILE = "terminal_os.wasm";
    private static Path wasmBinPath;

    /**
     * Initializes the WASM manager by creating the wasm-bin directory
     * and extracting bundled WASM files from the JAR.
     */
    public static void initialize() {
        wasmBinPath = Path.of(WASM_BIN_FOLDER);

        try {
            if (!Files.exists(wasmBinPath)) {
                Files.createDirectories(wasmBinPath);
                EvansComputerMod.LOGGER.info("Created wasm-bin/ directory at: {}", wasmBinPath.toAbsolutePath());
            } else {
                EvansComputerMod.LOGGER.info("wasm-bin/ directory exists at: {}", wasmBinPath.toAbsolutePath());
            }
            extractBundledWasmFiles();
        } catch (IOException e) {
            EvansComputerMod.LOGGER.error("Failed to create wasm-bin/ directory", e);
        }
    }

    /**
     * Extracts all bundled WASM files from the JAR to the wasm-bin directory.
     * Reads a manifest file (wasm-bin/manifest.txt) listing all WASM filenames,
     * then extracts each via getResourceAsStream (NeoForge classloader compatible).
     */
    private static void extractBundledWasmFiles() {
        int extracted = 0;

        // Try to read the manifest listing all WASM files
        List<String> wasmFiles = new ArrayList<>();
        try (InputStream manifestIs = WasmManager.class.getResourceAsStream("/wasm-bin/manifest.txt")) {
            if (manifestIs != null) {
                String content = new String(manifestIs.readAllBytes(), java.nio.charset.StandardCharsets.UTF_8);
                for (String line : content.split("\n")) {
                    String name = line.trim();
                    if (!name.isEmpty() && name.endsWith(".wasm")) {
                        wasmFiles.add(name);
                    }
                }
                EvansComputerMod.LOGGER.info("WASM manifest lists {} files", wasmFiles.size());
            } else {
                EvansComputerMod.LOGGER.warn("No WASM manifest found, extracting core file only");
                wasmFiles.add(CORE_WASM_FILE);
            }
        } catch (IOException e) {
            EvansComputerMod.LOGGER.warn("Failed to read WASM manifest", e);
            wasmFiles.add(CORE_WASM_FILE);
        }

        // Extract each file via getResourceAsStream (works in all classloader environments)
        for (String fileName : wasmFiles) {
            String resourcePath = "/wasm-bin/" + fileName;
            try (InputStream is = WasmManager.class.getResourceAsStream(resourcePath)) {
                if (is != null) {
                    Path target = wasmBinPath.resolve(fileName);
                    Files.copy(is, target, StandardCopyOption.REPLACE_EXISTING);
                    extracted++;
                } else {
                    EvansComputerMod.LOGGER.debug("WASM resource not found in JAR: {}", resourcePath);
                }
            } catch (IOException e) {
                EvansComputerMod.LOGGER.error("Failed to extract WASM file: {}", fileName, e);
            }
        }

        EvansComputerMod.LOGGER.info("Extracted {} WASM files to {}", extracted, wasmBinPath.toAbsolutePath());
    }

    /** Extract a single WASM file from JAR resources. */
    private static void extractSingleFile(String fileName) {
        String resourcePath = "/wasm-bin/" + fileName;
        try (InputStream is = WasmManager.class.getResourceAsStream(resourcePath)) {
            if (is != null) {
                Path target = wasmBinPath.resolve(fileName);
                Files.copy(is, target, StandardCopyOption.REPLACE_EXISTING);
                EvansComputerMod.LOGGER.info("Extracted WASM file: {}", fileName);
            }
        } catch (IOException e) {
            EvansComputerMod.LOGGER.error("Failed to extract WASM file: {}", fileName, e);
        }
    }
    
    /**
     * Gets the path to the wasm-bin directory.
     */
    public static Path getWasmBinPath() {
        return wasmBinPath;
    }
    
    /**
     * Executes a function from a WASM module.
     * 
     * @param fileName The name of the WASM file (with or without .wasm extension)
     * @param functionName The name of the function to execute
     * @param params The parameters to pass to the function (as strings, will be parsed as integers)
     * @return The result of the function execution
     * @throws WasmExecutionException If an error occurs during execution
     */
    public static WasmResult executeFunction(String fileName, String functionName, List<String> params) 
            throws WasmExecutionException {
        
        // Ensure .wasm extension
        if (!fileName.endsWith(".wasm")) {
            fileName = fileName + ".wasm";
        }
        
        Path wasmFile = wasmBinPath.resolve(fileName);
        
        if (!Files.exists(wasmFile)) {
            throw new WasmExecutionException("WASM file not found: " + wasmFile.toAbsolutePath());
        }
        
        try (Store<Void> store = Store.withoutData();
             Engine engine = store.engine();
             Module module = Module.fromFile(engine, wasmFile.toString());
             Instance instance = new Instance(store, module, List.of())) {
            
            // Get the function from the module's exports
            Func func = instance.getFunc(store, functionName)
                    .orElseThrow(() -> new WasmExecutionException(
                            "Function '" + functionName + "' not found in module"));
            
            // Parse parameters - try to infer types from the string values
            Val[] wasmParams = parseParameters(params);
            
            // Execute the function
            Val[] results = func.call(store, wasmParams);
            
            return new WasmResult(results);
            
        } catch (WasmtimeException e) {
            throw new WasmExecutionException("WASM execution error: " + e.getMessage(), e);
        } catch (WasmExecutionException e) {
            throw e;
        } catch (Exception e) {
            throw new WasmExecutionException("Error executing WASM function: " + e.getMessage(), e);
        }
    }
    
    /**
     * Lists all available WASM files in the wasm-bin directory.
     */
    public static List<String> listWasmFiles() {
        List<String> files = new ArrayList<>();
        
        if (wasmBinPath == null || !Files.exists(wasmBinPath)) {
            return files;
        }
        
        try (var stream = Files.list(wasmBinPath)) {
            stream.filter(path -> path.toString().endsWith(".wasm"))
                  .map(path -> path.getFileName().toString())
                  .forEach(files::add);
        } catch (IOException e) {
            EvansComputerMod.LOGGER.error("Failed to list WASM files", e);
        }
        
        return files;
    }
    
    /**
     * Parses string parameters into WASM Val objects.
     * Attempts to parse as integer first, then long, then float, then double.
     */
    private static Val[] parseParameters(List<String> params) throws WasmExecutionException {
        Val[] wasmParams = new Val[params.size()];
        
        for (int i = 0; i < params.size(); i++) {
            String param = params.get(i).trim();
            
            try {
                // Try to parse as the most specific type first
                if (param.contains(".")) {
                    // Floating point number
                    if (param.endsWith("f") || param.endsWith("F")) {
                        wasmParams[i] = Val.fromF32(Float.parseFloat(param.substring(0, param.length() - 1)));
                    } else {
                        wasmParams[i] = Val.fromF64(Double.parseDouble(param));
                    }
                } else {
                    // Integer type
                    if (param.endsWith("L") || param.endsWith("l")) {
                        wasmParams[i] = Val.fromI64(Long.parseLong(param.substring(0, param.length() - 1)));
                    } else {
                        try {
                            wasmParams[i] = Val.fromI32(Integer.parseInt(param));
                        } catch (NumberFormatException e) {
                            // Try as long if too big for int
                            wasmParams[i] = Val.fromI64(Long.parseLong(param));
                        }
                    }
                }
            } catch (NumberFormatException e) {
                throw new WasmExecutionException(
                    String.format("Failed to parse parameter %d: '%s' - not a valid number", i + 1, param)
                );
            }
        }
        
        return wasmParams;
    }
    
    /**
     * Represents the result of a WASM function execution.
     */
    public static class WasmResult {
        private final Val[] values;
        
        public WasmResult(Val[] values) {
            this.values = values;
        }
        
        public Val[] getValues() {
            return values;
        }
        
        public boolean hasResult() {
            return values != null && values.length > 0;
        }
        
        @Override
        public String toString() {
            if (!hasResult()) {
                return "(no return value)";
            }
            
            StringBuilder sb = new StringBuilder();
            for (int i = 0; i < values.length; i++) {
                if (i > 0) sb.append(", ");
                Val val = values[i];
                sb.append(formatValue(val));
            }
            return sb.toString();
        }
        
        private String formatValue(Val val) {
            return switch (val.getType()) {
                case I32 -> String.valueOf(val.i32());
                case I64 -> String.valueOf(val.i64());
                case F32 -> String.valueOf(val.f32());
                case F64 -> String.valueOf(val.f64());
                default -> val.toString();
            };
        }
    }
    
    /**
     * Exception thrown when WASM execution fails.
     */
    public static class WasmExecutionException extends Exception {
        public WasmExecutionException(String message) {
            super(message);
        }
        
        public WasmExecutionException(String message, Throwable cause) {
            super(message, cause);
        }
    }
}
