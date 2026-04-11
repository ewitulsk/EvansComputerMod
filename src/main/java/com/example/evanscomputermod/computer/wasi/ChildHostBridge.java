package com.example.evanscomputermod.computer.wasi;

import com.example.evanscomputermod.computer.ComputerInstance;

/**
 * Thin facade exposing kernel-side host operations (redstone, peripherals, sleep)
 * to child WASI processes. Each child WASI process gets host functions registered
 * in {@link WasiFunctions} that delegate through this bridge to the parent
 * {@link ComputerInstance}, which already implements these operations for the
 * kernel WASM.
 *
 * <p>The bridge doesn't touch WASM memory directly — it accepts/returns plain
 * Java values. The WASI handlers are responsible for marshalling between WASM
 * memory and these calls.
 */
public class ChildHostBridge {

    private final ComputerInstance parent;

    public ChildHostBridge(ComputerInstance parent) {
        this.parent = parent;
    }

    // --- Redstone ---

    /** Set redstone output power on a relative side (0-5), power 0-15. Returns 0 on success. */
    public int redstoneSetOutput(int side, int power) {
        return parent.bridgeRedstoneSetOutput(side, power);
    }

    /** Read redstone input power on a relative side (0-5). Returns 0-15. */
    public int redstoneGetInput(int side) {
        return parent.bridgeRedstoneGetInput(side);
    }

    /** Read all 6 sides' redstone input power into the given array (length 6). */
    public int redstoneGetAllInput(int[] out) {
        return parent.bridgeRedstoneGetAllInput(out);
    }

    // --- Peripherals ---

    /** List all connected peripherals as JSON. */
    public String peripheralListJson() {
        return parent.bridgePeripheralListJson();
    }

    /** Get methods for a peripheral as JSON. */
    public String peripheralMethodsJson(String name) {
        return parent.bridgePeripheralMethodsJson(name);
    }

    /** Call a peripheral method with JSON args, return result JSON. */
    public String peripheralCall(String name, String method, String argsJson) {
        return parent.bridgePeripheralCall(name, method, argsJson);
    }

    // --- Sleep ---

    /** Sleep for the given number of milliseconds (clamped 0..60000). */
    public void sleepMs(int ms) {
        parent.bridgeSleepMs(ms);
    }
}
