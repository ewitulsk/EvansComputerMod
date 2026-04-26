package com.example.evanscomputermod.wasm;

import com.example.evanscomputermod.api.wasm.WasmHostFunc;
import com.example.evanscomputermod.api.wasm.WasmInstance;
import com.example.evanscomputermod.api.wasm.WasmModuleHandle;
import com.example.evanscomputermod.api.wasm.WasmRuntime;
import com.example.evanscomputermod.api.wasm.WasmTrap;
import com.example.evanscomputermod.api.wasm.WasmValType;
import com.example.evanscomputermod.wasm.chicory.ChicoryRuntimeProvider;

import org.junit.jupiter.api.Test;

import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertNotNull;
import static org.junit.jupiter.api.Assumptions.assumeTrue;

/**
 * Smoke tests for the runtime SPI. The pre-migration version of this file
 * exercised wasmtime directly; the new SPI is provider-agnostic, so we drive
 * the Chicory provider through {@link com.example.evanscomputermod.api.wasm.WasmRuntime}
 * and let it speak for both implementations.
 */
public class WasmImportTest {

    private static WasmRuntime runtime() {
        return new ChicoryRuntimeProvider().create();
    }

    @Test
    public void simpleModuleLoadsAndAdds() {
        Path wasmFile = Path.of("wasm-examples/simple/target/wasm32-unknown-unknown/release/simple.wasm");
        assumeTrue(Files.exists(wasmFile), "simple.wasm not present (build the example first)");

        try {
            byte[] bytes = Files.readAllBytes(wasmFile);
            WasmRuntime rt = runtime();
            WasmModuleHandle module = rt.compile(bytes);
            try (WasmInstance instance = rt.instantiate(module, List.of())) {
                long[] result = instance.callExport("add", 2, 3);
                assertNotNull(result);
                assertEquals(5, (int) result[0]);
            }
        } catch (Exception e) {
            throw new RuntimeException(e);
        }
    }

    @Test
    public void terminalOsLoadsWithStubbedImports() {
        Path wasmFile = Path.of("wasm-bin/terminal_os.wasm");
        assumeTrue(Files.exists(wasmFile), "terminal_os.wasm not present (run scripts/build-wasm-programs.sh)");

        try {
            byte[] bytes = Files.readAllBytes(wasmFile);
            WasmRuntime rt = runtime();
            WasmModuleHandle module = rt.compile(bytes);

            // We don't supply any host functions — the SPI auto-stubs every
            // declared import with a no-op zero-returning function. This
            // proves the module's import surface can at least be linked.
            List<WasmHostFunc> imports = new ArrayList<>();
            try (WasmInstance instance = rt.instantiate(module, imports)) {
                assertNotNull(instance);
            }
        } catch (Exception e) {
            throw new RuntimeException(e);
        }
    }

    /**
     * Sanity-check the host-function bridge: WASM module imports a single
     * function, calls it, and returns whatever the host returned + 1.
     * (Skipped if no test fixture is available — this is a placeholder for
     * future smoke testing once we have a tiny WAT-built fixture in
     * src/test/resources.)
     */
    @Test
    public void hostFunctionRoundTrip() {
        Path fixture = Path.of("src/test/resources/host-roundtrip.wasm");
        assumeTrue(Files.exists(fixture), "host-roundtrip.wasm fixture missing");

        try {
            byte[] bytes = Files.readAllBytes(fixture);
            WasmRuntime rt = runtime();
            WasmModuleHandle module = rt.compile(bytes);

            List<WasmHostFunc> imports = List.of(
                    new WasmHostFunc("env", "host_value",
                            List.of(), List.of(WasmValType.I32),
                            (inst, args) -> WasmHostFunc.retI32(41))
            );

            try (WasmInstance instance = rt.instantiate(module, imports)) {
                long[] result = instance.callExport("entry");
                assertEquals(42, (int) result[0]);
            }
        } catch (WasmTrap | java.io.IOException e) {
            throw new RuntimeException(e);
        }
    }
}
