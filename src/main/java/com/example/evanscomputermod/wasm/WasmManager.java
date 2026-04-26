package com.example.evanscomputermod.wasm;

import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.api.wasm.WasmHostFunc;
import com.example.evanscomputermod.api.wasm.WasmInstance;
import com.example.evanscomputermod.api.wasm.WasmModuleHandle;
import com.example.evanscomputermod.api.wasm.WasmRuntime;
import com.example.evanscomputermod.api.wasm.WasmTrap;

import java.io.IOException;
import java.io.InputStream;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.StandardCopyOption;
import java.util.ArrayList;
import java.util.List;

/**
 * Manages WASM module loading and function execution. Holds the singleton
 * {@link WasmRuntime} chosen at server start (see
 * {@code EvansComputerMod.onCommonSetup}) and exposes utilities for
 * extracting bundled .wasm files and running ad-hoc functions for the
 * {@code /wasm} command.
 */
public class WasmManager {

    private static final String WASM_BIN_FOLDER = "wasm-bin";
    /** Core WASM file that must always be extracted (the OS kernel). */
    private static final String CORE_WASM_FILE = "terminal_os.wasm";
    private static Path wasmBinPath;

    private static volatile WasmRuntime runtime;

    /** Bind the runtime selected by {@code WasmRuntimeRegistry.select()}. */
    public static void bind(WasmRuntime r) {
        runtime = r;
    }

    /** The active runtime, or {@code null} if not yet bound. */
    public static WasmRuntime runtime() {
        return runtime;
    }

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
     */
    private static void extractBundledWasmFiles() {
        int extracted = 0;

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

    /**
     * Gets the path to the wasm-bin directory.
     */
    public static Path getWasmBinPath() {
        return wasmBinPath;
    }

    /**
     * Executes a function from a WASM module. Used by the {@code /wasm} debug
     * command; runs in a fresh instance with no host imports.
     */
    public static WasmResult executeFunction(String fileName, String functionName, List<String> params)
            throws WasmExecutionException {

        if (runtime == null) {
            throw new WasmExecutionException("No WASM runtime bound. Call WasmManager.bind() at startup.");
        }

        if (!fileName.endsWith(".wasm")) {
            fileName = fileName + ".wasm";
        }

        Path wasmFile = wasmBinPath.resolve(fileName);
        if (!Files.exists(wasmFile)) {
            throw new WasmExecutionException("WASM file not found: " + wasmFile.toAbsolutePath());
        }

        try {
            byte[] bytes = Files.readAllBytes(wasmFile);
            WasmModuleHandle module = runtime.compile(bytes);
            try (WasmInstance instance = runtime.instantiate(module, List.of())) {
                long[] args = parseParameters(params);
                long[] results = instance.callExport(functionName, args);
                return new WasmResult(results);
            }
        } catch (WasmTrap e) {
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
        if (wasmBinPath == null || !Files.exists(wasmBinPath)) return files;
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
     * Parses string parameters into i32/i64 longs. Floats are bit-cast.
     */
    private static long[] parseParameters(List<String> params) throws WasmExecutionException {
        long[] out = new long[params.size()];
        for (int i = 0; i < params.size(); i++) {
            String param = params.get(i).trim();
            try {
                if (param.contains(".")) {
                    if (param.endsWith("f") || param.endsWith("F")) {
                        out[i] = Integer.toUnsignedLong(
                                Float.floatToRawIntBits(Float.parseFloat(param.substring(0, param.length() - 1))));
                    } else {
                        out[i] = Double.doubleToRawLongBits(Double.parseDouble(param));
                    }
                } else if (param.endsWith("L") || param.endsWith("l")) {
                    out[i] = Long.parseLong(param.substring(0, param.length() - 1));
                } else {
                    try {
                        out[i] = Integer.parseInt(param);
                    } catch (NumberFormatException e) {
                        out[i] = Long.parseLong(param);
                    }
                }
            } catch (NumberFormatException e) {
                throw new WasmExecutionException(
                        String.format("Failed to parse parameter %d: '%s' - not a valid number", i + 1, param));
            }
        }
        return out;
    }

    /**
     * Result of a WASM function execution. Values are encoded per
     * {@link WasmHostFunc} conventions; consumers that care about exact types
     * should consult the called function's signature.
     */
    public static class WasmResult {
        private final long[] values;

        public WasmResult(long[] values) {
            this.values = values;
        }

        public long[] getValues() {
            return values;
        }

        public boolean hasResult() {
            return values != null && values.length > 0;
        }

        @Override
        public String toString() {
            if (!hasResult()) return "(no return value)";
            StringBuilder sb = new StringBuilder();
            for (int i = 0; i < values.length; i++) {
                if (i > 0) sb.append(", ");
                sb.append(values[i]);
            }
            return sb.toString();
        }
    }

    public static class WasmExecutionException extends Exception {
        public WasmExecutionException(String message) {
            super(message);
        }
        public WasmExecutionException(String message, Throwable cause) {
            super(message, cause);
        }
    }
}
