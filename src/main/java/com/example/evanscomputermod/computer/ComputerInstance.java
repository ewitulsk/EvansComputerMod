package com.example.evanscomputermod.computer;

import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.api.IComputerHost;
import com.example.evanscomputermod.api.IRedstoneProvider;
import com.example.evanscomputermod.api.ITerminalOutput;
import com.example.evanscomputermod.api.IWorldAccess;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
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
import java.util.concurrent.BlockingQueue;
import java.util.concurrent.ConcurrentLinkedQueue;
import java.util.concurrent.LinkedBlockingQueue;
import java.util.concurrent.TimeUnit;
import java.util.stream.Collectors;

import com.example.evanscomputermod.block.TerminalBlockEntity;
import com.example.evanscomputermod.api.ComputerModuleRegistry;
import com.example.evanscomputermod.wasm.ModuleMethodInvoker;
import com.example.evanscomputermod.wasm.PeripheralManager;
import com.example.evanscomputermod.wasm.PeripheralMethodInvoker;
import com.example.evanscomputermod.wasm.WasmManager;

/**
 * Provides host functions for WASM modules running in a computer context.
 * Allows WASM modules to write to the terminal display and access the file system.
 */
public class ComputerInstance implements AutoCloseable {

    // Directory for storing computer files (in game directory)
    private static final String COMPUTER_DATA_FOLDER = "computer-data";

    private final IComputerHost host;
    private final Engine engine;
    private final Store<Void> store;
    private final Path computerStoragePath;

    private Instance instance;
    private Memory memory;

    // Flag to indicate the WASM module has crashed and should not be used
    private volatile boolean faulted = false;

    // Flag to signal WASM execution should be interrupted (e.g., Ctrl+T or block break)
    private volatile boolean interrupted = false;

    // Worker thread for async WASM execution
    private Thread workerThread;
    private volatile boolean shutdownRequested = false;
    private final BlockingQueue<String> inputQueue = new LinkedBlockingQueue<>();

    // Interrupt system
    private static final int INTERRUPT_BUFFER_ADDR = 0x11000;
    private final ConcurrentLinkedQueue<InterruptEvent> interruptQueue = new ConcurrentLinkedQueue<>();
    private volatile boolean wasmExecuting = false;
    private int lastInterruptPayloadLen = 0;

    /**
     * An interrupt event queued for delivery to WASM.
     */
    private static class InterruptEvent {
        final int irq;
        final String payload;

        InterruptEvent(int irq, String payload) {
            this.irq = irq;
            this.payload = payload;
        }
    }

    // Counter for wasm-bindgen object reference handles
    private final java.util.concurrent.atomic.AtomicInteger nextObjectHandle = new java.util.concurrent.atomic.AtomicInteger(1);

    // Host functions (need to keep references to prevent GC)
    private final List<Func> hostFunctions = new ArrayList<>();
    private final Map<String, Extern> hostFunctionMap = new HashMap<>();

    // Network: MAC addresses derived from computerId (one per face: down=0, up=1, north=2, south=3, west=4, east=5)
    private byte[][] networkMacs;

    public ComputerInstance(IComputerHost host, byte[][] macs) {
        this.host = host;
        // IMPORTANT: Store.withoutData() creates its own Engine internally.
        // We MUST use store.engine() for Module.fromFile(), otherwise we get
        // "cross-Engine instantiation is not currently supported" error!
        this.store = Store.withoutData();
        this.engine = store.engine();

        // Set up computer storage directory
        UUID computerId = host.getComputerId();
        this.computerStoragePath = Path.of(COMPUTER_DATA_FOLDER, computerId.toString());
        try {
            Files.createDirectories(computerStoragePath);
            EvansComputerMod.LOGGER.info("Computer storage path: {}", computerStoragePath.toAbsolutePath());
        } catch (IOException e) {
            EvansComputerMod.LOGGER.error("Failed to create computer storage directory", e);
        }

        // Use provided MAC list (6 built-in + any from attached InterfaceBlocks)
        this.networkMacs = macs;

        // Register all NICs on the network hub if available
        NetworkHub hub = NetworkHub.getInstance();
        if (hub != null) {
            for (byte[] mac : this.networkMacs) {
                hub.registerNic(mac, this::queueInterrupt);
            }
        }

        // Create all host functions
        createHostFunctions();
    }

    /**
     * Private helper to write text to the terminal output, with null check.
     */
    private void writeToTerminal(String text) {
        ITerminalOutput output = host.getTerminalOutput();
        if (output != null) {
            output.write(text);
        }
    }

    /**
     * Starts the worker thread that processes WASM input asynchronously.
     * This allows the main server thread to remain responsive even if WASM enters an infinite loop.
     */
    public void startWorkerThread() {
        if (workerThread != null && workerThread.isAlive()) {
            return;  // Already running
        }

        shutdownRequested = false;
        workerThread = new Thread(this::workerLoop, "WASM-Worker-" + host.getComputerId().toString().substring(0, 8));
        workerThread.setDaemon(true);
        workerThread.start();
        EvansComputerMod.LOGGER.info("Started WASM worker thread: {}", workerThread.getName());
    }

    /**
     * Worker thread main loop - processes input and interrupts from the queues.
     */
    private void workerLoop() {
        EvansComputerMod.LOGGER.debug("WASM worker thread started");

        while (!shutdownRequested && !Thread.currentThread().isInterrupted()) {
            try {
                // Drain pending interrupts before processing input
                drainAndDeliverInterrupts();

                // Wait for input with timeout to allow checking shutdown flag
                String input = inputQueue.poll(100, TimeUnit.MILLISECONDS);

                if (input != null) {
                    processInputOnWorker(input);
                    // Drain interrupts that arrived during input processing
                    drainAndDeliverInterrupts();
                }
            } catch (InterruptedException e) {
                // Thread was interrupted, exit gracefully
                Thread.currentThread().interrupt();
                break;
            } catch (Throwable e) {
                // Log any unexpected errors but keep the worker running
                EvansComputerMod.LOGGER.error("Error in WASM worker thread", e);
            }
        }

        EvansComputerMod.LOGGER.debug("WASM worker thread exiting");
    }

    /**
     * Processes input on the worker thread - calls the WASM input handler.
     */
    private void processInputOnWorker(String input) {
        if (instance == null || faulted) {
            return;
        }

        Optional<Func> inputHandler = instance.getFunc(store, "on_input");
        if (inputHandler.isEmpty()) {
            inputHandler = instance.getFunc(store, "handle_input");
        }

        if (inputHandler.isPresent() && memory != null) {
            wasmExecuting = true;
            try {
                // Write the input string to WASM memory
                byte[] bytes = input.getBytes(StandardCharsets.UTF_8);
                ByteBuffer buffer = memory.buffer(store);

                // Use a fixed input buffer location (at address 0x10000)
                int inputBufferAddr = 0x10000;
                buffer.position(inputBufferAddr);
                buffer.put(bytes);

                // Call the input handler with pointer and length
                inputHandler.get().call(store, Val.fromI32(inputBufferAddr), Val.fromI32(bytes.length));

                // After successful execution, sync terminal state to clients
                syncTerminalToClients();

            } catch (WasmInterruptedException e) {
                // Interrupted execution - clear flag so OS can receive Ctrl+T and reset to shell
                EvansComputerMod.LOGGER.info("WASM execution was interrupted");
                interrupted = false;
                // Clear thread's interrupted flag so worker loop continues
                Thread.interrupted();
                syncTerminalToClients();
            } catch (Throwable e) {
                // Check if this was caused by an interrupt
                if (interrupted) {
                    // Clear flag so OS can receive Ctrl+T and reset to shell
                    EvansComputerMod.LOGGER.info("WASM execution was interrupted (via exception)");
                    interrupted = false;
                    // Clear thread's interrupted flag so worker loop continues
                    Thread.interrupted();
                    syncTerminalToClients();
                    return;
                }

                // Mark as faulted to prevent further use of corrupted WASM state
                faulted = true;
                EvansComputerMod.LOGGER.error("Error in WASM execution", e);
                writeToTerminal("\nWASM Error: " + e.getMessage() + "\n");
                writeToTerminal("[Terminal faulted - close and reopen to reset]\n");
                syncTerminalToClients();
            } finally {
                wasmExecuting = false;
            }
        }
    }

    /**
     * Returns whether WASM is currently executing a call (for interrupt queueing decisions).
     */
    public boolean isWasmExecuting() {
        return wasmExecuting;
    }

    /**
     * Queues an interrupt event for delivery to WASM on the worker thread.
     * Thread-safe - can be called from any thread.
     *
     * @param irq     The interrupt number (1=KEYBOARD, 2=REDSTONE, 15=TERMINATE)
     * @param payload JSON-encoded data for the interrupt handler
     */
    public void queueInterrupt(int irq, String payload) {
        interruptQueue.offer(new InterruptEvent(irq, payload));
    }

    /**
     * Drains the interrupt queue and delivers each event to WASM via on_interrupt().
     * Must only be called from the worker thread.
     */
    private void drainAndDeliverInterrupts() {
        if (instance == null || faulted || memory == null) return;

        boolean delivered = false;
        InterruptEvent evt;
        while ((evt = interruptQueue.poll()) != null) {
            deliverInterrupt(evt);
            delivered = true;
        }
        if (delivered) {
            syncTerminalToClients();
        }
    }

    /**
     * Delivers a single interrupt event to WASM by calling the on_interrupt export.
     * Writes the payload to WASM memory at INTERRUPT_BUFFER_ADDR and calls on_interrupt(irq, ptr, len).
     */
    private void deliverInterrupt(InterruptEvent evt) {
        Optional<Func> handler = instance.getFunc(store, "on_interrupt");
        if (handler.isEmpty()) return;

        try {
            byte[] payloadBytes = evt.payload.getBytes(StandardCharsets.UTF_8);
            ByteBuffer buffer = memory.buffer(store);
            buffer.position(INTERRUPT_BUFFER_ADDR);
            buffer.put(payloadBytes);

            handler.get().call(store,
                    Val.fromI32(evt.irq),
                    Val.fromI32(INTERRUPT_BUFFER_ADDR),
                    Val.fromI32(payloadBytes.length));
        } catch (WasmInterruptedException e) {
            EvansComputerMod.LOGGER.info("Interrupt delivery was interrupted");
            interrupted = false;
            Thread.interrupted();
        } catch (Throwable e) {
            if (interrupted) {
                interrupted = false;
                Thread.interrupted();
            } else {
                EvansComputerMod.LOGGER.error("Error delivering interrupt IRQ={}", evt.irq, e);
            }
        }
    }

    /**
     * Syncs the terminal buffer to connected clients.
     * Called from the worker thread after WASM modifies the terminal.
     */
    private void syncTerminalToClients() {
        host.syncToClients();
    }

    /**
     * Creates all host functions and stores them in a map for lookup by name.
     */
    private void createHostFunctions() {
        // terminal_write(ptr: i32, len: i32) -> i32
        // Use new Func() pattern instead of WasmFunctions.wrap() to ensure exceptions propagate
        Func terminalWriteFunc = new Func(store,
                new FuncType(new Type[]{Type.I32, Type.I32}, new Type[]{Type.I32}),
                (caller, params, results) -> {
                    EvansComputerMod.LOGGER.debug("terminal_write callback invoked, interrupted={}", interrupted);
                    try {
                        int result = hostTerminalWrite(params[0].i32(), params[1].i32());
                        results[0] = Val.fromI32(result);
                    } catch (WasmInterruptedException e) {
                        EvansComputerMod.LOGGER.info("WasmInterruptedException caught in terminal_write callback - rethrowing");
                        throw e;
                    }
                });
        hostFunctions.add(terminalWriteFunc);
        hostFunctionMap.put("terminal_write", Extern.fromFunc(terminalWriteFunc));

        // terminal_clear() -> void
        // Create a function with no parameters and no return values
        Func terminalClearFunc = new Func(store, new FuncType(new Type[]{}, new Type[]{}),
                (caller, params, results) -> {
                    checkInterrupted();
                    ITerminalOutput output = host.getTerminalOutput();
                    if (output != null) {
                        output.clearBuffer();
                    }
                });
        hostFunctions.add(terminalClearFunc);
        hostFunctionMap.put("terminal_clear", Extern.fromFunc(terminalClearFunc));

        // terminal_set_cursor(x: i32, y: i32) -> void
        // Create a function with two i32 parameters and no return values
        Func terminalSetCursorFunc = new Func(store,
                new FuncType(new Type[]{Type.I32, Type.I32}, new Type[]{}),
                (caller, params, results) -> {
                    checkInterrupted();
                    int x = params[0].i32();
                    int y = params[1].i32();
                    ITerminalOutput output = host.getTerminalOutput();
                    if (output != null) {
                        output.setCursor(x, y);
                    }
                });
        hostFunctions.add(terminalSetCursorFunc);
        hostFunctionMap.put("terminal_set_cursor", Extern.fromFunc(terminalSetCursorFunc));

        // terminal_get_width() -> i32
        Func terminalGetWidthFunc = WasmFunctions.wrap(store, WasmValType.I32,
                () -> {
                    ITerminalOutput output = host.getTerminalOutput();
                    return output != null ? output.getWidth() : 80;
                });
        hostFunctions.add(terminalGetWidthFunc);
        hostFunctionMap.put("terminal_get_width", Extern.fromFunc(terminalGetWidthFunc));

        // terminal_get_height() -> i32
        Func terminalGetHeightFunc = WasmFunctions.wrap(store, WasmValType.I32,
                () -> {
                    ITerminalOutput output = host.getTerminalOutput();
                    return output != null ? output.getHeight() : 24;
                });
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

        // file_mkdir(path_ptr, path_len) -> 0 on success, -1 on error
        Func fileMkdirFunc = new Func(store,
                new FuncType(new Type[]{Type.I32, Type.I32}, new Type[]{Type.I32}),
                (caller, params, results) -> {
                    int pathPtr = params[0].i32();
                    int pathLen = params[1].i32();
                    results[0] = Val.fromI32(hostFileMkdir(pathPtr, pathLen));
                });
        hostFunctions.add(fileMkdirFunc);
        hostFunctionMap.put("file_mkdir", Extern.fromFunc(fileMkdirFunc));

        // file_is_dir(path_ptr, path_len) -> 1 if directory, 0 if not
        Func fileIsDirFunc = new Func(store,
                new FuncType(new Type[]{Type.I32, Type.I32}, new Type[]{Type.I32}),
                (caller, params, results) -> {
                    int pathPtr = params[0].i32();
                    int pathLen = params[1].i32();
                    results[0] = Val.fromI32(hostFileIsDir(pathPtr, pathLen));
                });
        hostFunctions.add(fileIsDirFunc);
        hostFunctionMap.put("file_is_dir", Extern.fromFunc(fileIsDirFunc));

        // file_list_dir(path_ptr, path_len, buf_ptr, buf_len) -> bytes written, or -1 on error
        Func fileListDirFunc = new Func(store,
                new FuncType(new Type[]{Type.I32, Type.I32, Type.I32, Type.I32}, new Type[]{Type.I32}),
                (caller, params, results) -> {
                    int pathPtr = params[0].i32();
                    int pathLen = params[1].i32();
                    int bufPtr = params[2].i32();
                    int bufLen = params[3].i32();
                    results[0] = Val.fromI32(hostFileListDir(pathPtr, pathLen, bufPtr, bufLen));
                });
        hostFunctions.add(fileListDirFunc);
        hostFunctionMap.put("file_list_dir", Extern.fromFunc(fileListDirFunc));

        // === Redstone Output ===

        // redstone_set_output(side: i32, power: i32) -> i32
        Func redstoneSetOutputFunc = new Func(store,
                new FuncType(new Type[]{Type.I32, Type.I32}, new Type[]{Type.I32}),
                (caller, params, results) -> {
                    int side = params[0].i32();
                    int power = params[1].i32();
                    results[0] = Val.fromI32(hostRedstoneSetOutput(side, power));
                });
        hostFunctions.add(redstoneSetOutputFunc);
        hostFunctionMap.put("redstone_set_output", Extern.fromFunc(redstoneSetOutputFunc));

        // === Sleep Function ===

        // sleep_ms(milliseconds: i32) -> void
        Func sleepMsFunc = new Func(store,
                new FuncType(new Type[]{Type.I32}, new Type[]{}),
                (caller, params, results) -> {
                    int ms = params[0].i32();
                    hostSleepMs(ms);
                });
        hostFunctions.add(sleepMsFunc);
        hostFunctionMap.put("sleep_ms", Extern.fromFunc(sleepMsFunc));

        // === Line Input Function ===

        // terminal_read_line(prompt_ptr: i32, prompt_len: i32, buf_ptr: i32, buf_len: i32) -> i32
        Func readLineFunc = new Func(store,
                new FuncType(new Type[]{Type.I32, Type.I32, Type.I32, Type.I32}, new Type[]{Type.I32}),
                (caller, params, results) -> {
                    int promptPtr = params[0].i32();
                    int promptLen = params[1].i32();
                    int bufPtr = params[2].i32();
                    int bufLen = params[3].i32();
                    results[0] = Val.fromI32(hostReadLine(promptPtr, promptLen, bufPtr, bufLen));
                });
        hostFunctions.add(readLineFunc);
        hostFunctionMap.put("terminal_read_line", Extern.fromFunc(readLineFunc));

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

        // === CC:Tweaked Peripheral Integration ===
        // peripheral_list(buf_ptr: i32, buf_len: i32) -> i32 (bytes written, or -1 on error)
        Func peripheralListFunc = new Func(store,
                new FuncType(new Type[]{Type.I32, Type.I32}, new Type[]{Type.I32}),
                (caller, params, results) -> {
                    int bufPtr = params[0].i32();
                    int bufLen = params[1].i32();
                    results[0] = Val.fromI32(hostPeripheralList(bufPtr, bufLen));
                });
        hostFunctions.add(peripheralListFunc);
        hostFunctionMap.put("peripheral_list", Extern.fromFunc(peripheralListFunc));

        // peripheral_get_methods(name_ptr, name_len, buf_ptr, buf_len) -> i32 (bytes written, or -1 on error)
        Func peripheralGetMethodsFunc = new Func(store,
                new FuncType(new Type[]{Type.I32, Type.I32, Type.I32, Type.I32}, new Type[]{Type.I32}),
                (caller, params, results) -> {
                    int namePtr = params[0].i32();
                    int nameLen = params[1].i32();
                    int bufPtr = params[2].i32();
                    int bufLen = params[3].i32();
                    results[0] = Val.fromI32(hostPeripheralGetMethods(namePtr, nameLen, bufPtr, bufLen));
                });
        hostFunctions.add(peripheralGetMethodsFunc);
        hostFunctionMap.put("peripheral_get_methods", Extern.fromFunc(peripheralGetMethodsFunc));

        // peripheral_call(name_ptr, name_len, method_ptr, method_len, args_ptr, args_len, result_ptr, result_len) -> i32
        Func peripheralCallFunc = new Func(store,
                new FuncType(new Type[]{Type.I32, Type.I32, Type.I32, Type.I32, Type.I32, Type.I32, Type.I32, Type.I32}, new Type[]{Type.I32}),
                (caller, params, results) -> {
                    int namePtr = params[0].i32();
                    int nameLen = params[1].i32();
                    int methodPtr = params[2].i32();
                    int methodLen = params[3].i32();
                    int argsPtr = params[4].i32();
                    int argsLen = params[5].i32();
                    int resultPtr = params[6].i32();
                    int resultLen = params[7].i32();
                    results[0] = Val.fromI32(hostPeripheralCall(namePtr, nameLen, methodPtr, methodLen, argsPtr, argsLen, resultPtr, resultLen));
                });
        hostFunctions.add(peripheralCallFunc);
        hostFunctionMap.put("peripheral_call", Extern.fromFunc(peripheralCallFunc));

        // === Redstone Input ===

        // redstone_get_input(side: i32) -> i32
        Func redstoneGetInputFunc = new Func(store,
                new FuncType(new Type[]{Type.I32}, new Type[]{Type.I32}),
                (caller, params, results) -> {
                    int side = params[0].i32();
                    results[0] = Val.fromI32(hostRedstoneGetInput(side));
                });
        hostFunctions.add(redstoneGetInputFunc);
        hostFunctionMap.put("redstone_get_input", Extern.fromFunc(redstoneGetInputFunc));

        // redstone_get_all_input(buf_ptr: i32) -> i32 (writes 6 i32 values, returns 0 on success)
        Func redstoneGetAllInputFunc = new Func(store,
                new FuncType(new Type[]{Type.I32}, new Type[]{Type.I32}),
                (caller, params, results) -> {
                    int bufPtr = params[0].i32();
                    results[0] = Val.fromI32(hostRedstoneGetAllInput(bufPtr));
                });
        hostFunctions.add(redstoneGetAllInputFunc);
        hostFunctionMap.put("redstone_get_all_input", Extern.fromFunc(redstoneGetAllInputFunc));

        // === Interrupt Support ===

        // interrupt_poll(buf_ptr: i32, buf_len: i32) -> i32 (returns IRQ number, or -1 if none)
        Func interruptPollFunc = new Func(store,
                new FuncType(new Type[]{Type.I32, Type.I32}, new Type[]{Type.I32}),
                (caller, params, results) -> {
                    int bufPtr = params[0].i32();
                    int bufLen = params[1].i32();
                    results[0] = Val.fromI32(hostInterruptPoll(bufPtr, bufLen));
                });
        hostFunctions.add(interruptPollFunc);
        hostFunctionMap.put("interrupt_poll", Extern.fromFunc(interruptPollFunc));

        // interrupt_poll_len() -> i32 (returns payload length of last polled interrupt)
        Func interruptPollLenFunc = new Func(store,
                new FuncType(new Type[]{}, new Type[]{Type.I32}),
                (caller, params, results) -> {
                    results[0] = Val.fromI32(lastInterruptPayloadLen);
                });
        hostFunctions.add(interruptPollLenFunc);
        hostFunctionMap.put("interrupt_poll_len", Extern.fromFunc(interruptPollLenFunc));

        // === Visual Editor ===
        // open_visual_editor() -> void
        Func openVisualEditorFunc = new Func(store, new FuncType(new Type[]{}, new Type[]{}),
                (caller, params, results) -> {
                    checkInterrupted();
                    if (host.getVisualProgramming() != null) {
                        host.getVisualProgramming().openVisualEditor();
                    }
                });
        hostFunctions.add(openVisualEditorFunc);
        hostFunctionMap.put("open_visual_editor", Extern.fromFunc(openVisualEditorFunc));

        // === Module Call Bridge (Annotation-Driven Auto-Registration) ===

        // module_call(module_ptr, module_len, method_ptr, method_len, args_ptr, args_len, result_ptr, result_len) -> i32
        Func moduleCallFunc = new Func(store,
                new FuncType(new Type[]{Type.I32, Type.I32, Type.I32, Type.I32, Type.I32, Type.I32, Type.I32, Type.I32}, new Type[]{Type.I32}),
                (caller, params, results) -> {
                    int modulePtr = params[0].i32();
                    int moduleLen = params[1].i32();
                    int methodPtr = params[2].i32();
                    int methodLen = params[3].i32();
                    int argsPtr = params[4].i32();
                    int argsLen = params[5].i32();
                    int resultPtr = params[6].i32();
                    int resultLen = params[7].i32();
                    results[0] = Val.fromI32(hostModuleCall(modulePtr, moduleLen, methodPtr, methodLen, argsPtr, argsLen, resultPtr, resultLen));
                });
        hostFunctions.add(moduleCallFunc);
        hostFunctionMap.put("module_call", Extern.fromFunc(moduleCallFunc));

        // module_list(buf_ptr, buf_len) -> i32
        Func moduleListFunc = new Func(store,
                new FuncType(new Type[]{Type.I32, Type.I32}, new Type[]{Type.I32}),
                (caller, params, results) -> {
                    int bufPtr = params[0].i32();
                    int bufLen = params[1].i32();
                    results[0] = Val.fromI32(hostModuleList(bufPtr, bufLen));
                });
        hostFunctions.add(moduleListFunc);
        hostFunctionMap.put("module_list", Extern.fromFunc(moduleListFunc));

        // === Network Host Functions (multi-interface) ===

        // net_get_interface_count() -> i32
        Func netGetInterfaceCountFunc = new Func(store,
                new FuncType(new Type[]{}, new Type[]{Type.I32}),
                (caller, params, results) -> {
                    results[0] = Val.fromI32(networkMacs.length);
                });
        hostFunctions.add(netGetInterfaceCountFunc);
        hostFunctionMap.put("net_get_interface_count", Extern.fromFunc(netGetInterfaceCountFunc));

        // net_get_interface_mac(index: i32, buf_ptr: i32) -> i32
        Func netGetInterfaceMacFunc = new Func(store,
                new FuncType(new Type[]{Type.I32, Type.I32}, new Type[]{Type.I32}),
                (caller, params, results) -> {
                    int index = params[0].i32();
                    int bufPtr = params[1].i32();
                    if (index < 0 || index >= networkMacs.length) {
                        results[0] = Val.fromI32(-1);
                        return;
                    }
                    writeBytesToMemory(networkMacs[index], bufPtr, 6);
                    results[0] = Val.fromI32(6);
                });
        hostFunctions.add(netGetInterfaceMacFunc);
        hostFunctionMap.put("net_get_interface_mac", Extern.fromFunc(netGetInterfaceMacFunc));

        // net_tx_frame_on(index: i32, buf_ptr: i32, frame_len: i32) -> i32
        Func netTxFrameOnFunc = new Func(store,
                new FuncType(new Type[]{Type.I32, Type.I32, Type.I32}, new Type[]{Type.I32}),
                (caller, params, results) -> {
                    int index = params[0].i32();
                    int bufPtr = params[1].i32();
                    int frameLen = params[2].i32();
                    if (index < 0 || index >= networkMacs.length || frameLen < 14 || frameLen > 1518) {
                        results[0] = Val.fromI32(-1);
                        return;
                    }
                    byte[] frame = readBytesFromMemory(bufPtr, frameLen);
                    if (frame.length == 0) {
                        results[0] = Val.fromI32(-1);
                        return;
                    }
                    NetworkHub hub = NetworkHub.getInstance();
                    if (hub != null) {
                        hub.transmit(networkMacs[index], frame);
                    }
                    results[0] = Val.fromI32(0);
                });
        hostFunctions.add(netTxFrameOnFunc);
        hostFunctionMap.put("net_tx_frame_on", Extern.fromFunc(netTxFrameOnFunc));

        // net_rx_frame_on(index: i32, buf_ptr: i32, buf_len: i32) -> i32
        Func netRxFrameOnFunc = new Func(store,
                new FuncType(new Type[]{Type.I32, Type.I32, Type.I32}, new Type[]{Type.I32}),
                (caller, params, results) -> {
                    int index = params[0].i32();
                    int bufPtr = params[1].i32();
                    int bufLen = params[2].i32();
                    if (index < 0 || index >= networkMacs.length) {
                        results[0] = Val.fromI32(-1);
                        return;
                    }
                    NetworkHub hub = NetworkHub.getInstance();
                    if (hub == null) {
                        results[0] = Val.fromI32(-1);
                        return;
                    }
                    byte[] frame = hub.receive(networkMacs[index]);
                    if (frame == null) {
                        results[0] = Val.fromI32(-1);
                        return;
                    }
                    int writeLen = Math.min(frame.length, bufLen);
                    writeBytesToMemory(frame, bufPtr, writeLen);
                    results[0] = Val.fromI32(writeLen);
                });
        hostFunctions.add(netRxFrameOnFunc);
        hostFunctionMap.put("net_rx_frame_on", Extern.fromFunc(netRxFrameOnFunc));

        // net_rx_frame_any(buf_ptr: i32, buf_len: i32, iface_idx_ptr: i32) -> i32
        Func netRxFrameAnyFunc = new Func(store,
                new FuncType(new Type[]{Type.I32, Type.I32, Type.I32}, new Type[]{Type.I32}),
                (caller, params, results) -> {
                    int bufPtr = params[0].i32();
                    int bufLen = params[1].i32();
                    int ifaceIdxPtr = params[2].i32();
                    NetworkHub hub = NetworkHub.getInstance();
                    if (hub == null) {
                        results[0] = Val.fromI32(-1);
                        return;
                    }
                    for (int i = 0; i < networkMacs.length; i++) {
                        byte[] frame = hub.receive(networkMacs[i]);
                        if (frame != null) {
                            int writeLen = Math.min(frame.length, bufLen);
                            writeBytesToMemory(frame, bufPtr, writeLen);
                            // Write interface index as little-endian i32
                            byte[] idxBytes = new byte[]{
                                (byte)(i & 0xFF), (byte)((i >> 8) & 0xFF),
                                (byte)((i >> 16) & 0xFF), (byte)((i >> 24) & 0xFF)
                            };
                            writeBytesToMemory(idxBytes, ifaceIdxPtr, 4);
                            results[0] = Val.fromI32(writeLen);
                            return;
                        }
                    }
                    results[0] = Val.fromI32(-1);
                });
        hostFunctions.add(netRxFrameAnyFunc);
        hostFunctionMap.put("net_rx_frame_any", Extern.fromFunc(netRxFrameAnyFunc));

        // net_set_promiscuous_on(index: i32, enabled: i32) -> i32
        Func netSetPromiscuousOnFunc = new Func(store,
                new FuncType(new Type[]{Type.I32, Type.I32}, new Type[]{Type.I32}),
                (caller, params, results) -> {
                    int index = params[0].i32();
                    int enabled = params[1].i32();
                    if (index < 0 || index >= networkMacs.length) {
                        results[0] = Val.fromI32(-1);
                        return;
                    }
                    NetworkHub hub = NetworkHub.getInstance();
                    if (hub != null) {
                        hub.setPromiscuous(networkMacs[index], enabled != 0);
                    }
                    results[0] = Val.fromI32(0);
                });
        hostFunctions.add(netSetPromiscuousOnFunc);
        hostFunctionMap.put("net_set_promiscuous_on", Extern.fromFunc(netSetPromiscuousOnFunc));

        // net_set_link_state(index: i32, up: i32) -> i32
        // Notifies the host that link state changed (for visual cable disconnect).
        Func netSetLinkStateFunc = new Func(store,
                new FuncType(new Type[]{Type.I32, Type.I32}, new Type[]{Type.I32}),
                (caller, params, results) -> {
                    int index = params[0].i32();
                    int up = params[1].i32();
                    if (index < 0 || index >= networkMacs.length) {
                        results[0] = Val.fromI32(-1);
                        return;
                    }
                    // Notify the terminal block entity to update visuals
                    if (host instanceof TerminalBlockEntity tbe) {
                        if (index < 6) {
                            tbe.setFaceDisabled(index, up == 0);
                            // Schedule block update on server thread
                            var server = host.getServer();
                            if (server != null) {
                                server.execute(() -> tbe.updateDisabledFaces(index, up != 0));
                            }
                        }
                    }
                    results[0] = Val.fromI32(0);
                });
        hostFunctions.add(netSetLinkStateFunc);
        hostFunctionMap.put("net_set_link_state", Extern.fromFunc(netSetLinkStateFunc));

        // === wasm-bindgen stubs ===
        // These are stubs for wasm-bindgen functions that RustPython's dependencies require.
        // Most of these are never actually called in our non-browser environment.
        createWasmBindgenStubs();

        EvansComputerMod.LOGGER.debug("Created {} host functions", hostFunctions.size());
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
        addStubVoid("__wbindgen_object_drop_ref", Type.I32);  // type 1: (i32) -> void - no-op, we don't track drops
        addObjectCloneRef("__wbindgen_object_clone_ref");  // type 4: (i32) -> i32 - return new handle

        // Date/time functions - properly implemented for chrono/time support
        addDateNew("__wbg_new_b2db8aa2650f793a");  // Date(timestamp) - type 4: (i32) -> i32
        addTimezoneOffset("__wbg_getTimezoneOffset_45389e26d6f46823");  // type 20: (i32) -> f64
        addDateNew0("__wbg_new_0_23cedd11d9b40c9d");  // Date() for now - type 7: () -> i32
        addGetTime("__wbg_getTime_ad1e9878a735af08");  // type 20: (i32) -> f64
        addDateNow("__wbg_now_2c70f2474e348581");  // type 21: () -> f64

        // Boolean checks - all type 4: (i32) -> i32
        addIsObject("__wbg___wbindgen_is_object_ce774f3490692386");  // Return true for non-zero handles
        addStubI32ReturnValue("__wbg___wbindgen_is_string_704ef9c8fc131030", 0, Type.I32);
        addStubI32ReturnValue("__wbg___wbindgen_is_function_8d400b8b1af978cd", 0, Type.I32);
        addStubI32ReturnValue("__wbg___wbindgen_is_undefined_f6b95eab589e0269", 1, Type.I32);  // Return true (undefined)

        // Crypto/random functions - return valid handles so getrandom can use them
        addCryptoObject("__wbg_crypto_574e78ad8b13b65f");  // type 4: (i32) -> i32
        addStubI32Return("__wbg_msCrypto_a61aeb35a24c1329", Type.I32);  // type 4: (i32) -> i32 - return 0 (no msCrypto)
        addRandomFillSync("__wbg_randomFillSync_ac0988aba3254290");  // type 16: (i32, i32) -> void
        addGetRandomValues("__wbg_getRandomValues_b8f5dbd5f3995a9e");  // type 16: (i32, i32) -> void

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

        // Array functions - return valid handles
        addUint8ArrayNew("__wbg_new_with_length_aa5eaf41d35235e5");  // type 4: (i32) -> i32
        addUint8ArraySubarray("__wbg_subarray_845f2f5bce7d061a");  // type 3: (i32, i32, i32) -> i32
        addUint8ArrayLength("__wbg_length_22ac23eaec9d8053");  // type 4: (i32) -> i32

        // Misc functions
        addStubI32Return("__wbg_new_no_args_cb138f77cf6151ee", Type.I32, Type.I32);  // type 2: (i32, i32) -> i32
        addStubVoid("__wbg_prototypesetcall_dfe9b766cdc1f1fd", Type.I32, Type.I32, Type.I32);  // type 0: (i32, i32, i32) -> void

        // Error handling functions - these need special implementations to read error messages
        addErrorHandler("__wbg_error_d01e9edc65d6e61f");  // console.error
        addThrowHandler("__wbg___wbindgen_throw_dd24417ed36fc46e");  // throw exception

        // Externref table functions
        addStubVoid("__wbindgen_externref_table_set_null", Type.I32);  // type 1: (i32) -> void - no-op
        addExternrefTableGrow("__wbindgen_externref_table_grow");  // type 4: (i32) -> i32
    }

    /**
     * Adds a special handler for __wbg_error that logs the error message from WASM memory.
     */
    private void addErrorHandler(String name) {
        Func func = new Func(store, new FuncType(new Type[]{Type.I32, Type.I32}, new Type[]{}),
                (caller, params, results) -> {
                    int ptr = params[0].i32();
                    int len = params[1].i32();
                    String msg = readStringFromMemory(ptr, len);
                    EvansComputerMod.LOGGER.error("WASM error ({}): {}", name, msg);
                    writeToTerminal("\n[WASM Error: " + msg + "]\n");
                });
        hostFunctions.add(func);
        hostFunctionMap.put(name, Extern.fromFunc(func));
    }

    /**
     * Adds a special handler for __wbindgen_throw that logs the error and throws an exception.
     */
    private void addThrowHandler(String name) {
        Func func = new Func(store, new FuncType(new Type[]{Type.I32, Type.I32}, new Type[]{}),
                (caller, params, results) -> {
                    int ptr = params[0].i32();
                    int len = params[1].i32();
                    String msg = readStringFromMemory(ptr, len);
                    EvansComputerMod.LOGGER.error("WASM throw ({}): {}", name, msg);
                    writeToTerminal("\n[WASM Throw: " + msg + "]\n");
                    throw new RuntimeException("WASM throw: " + msg);
                });
        hostFunctions.add(func);
        hostFunctionMap.put(name, Extern.fromFunc(func));
    }

    /**
     * Adds a stub function that takes parameters and returns void.
     */
    private void addStubVoid(String name, Type... paramTypes) {
        final String funcName = name;
        Func func = new Func(store, new FuncType(paramTypes, new Type[]{}),
                (caller, params, results) -> {
                    EvansComputerMod.LOGGER.trace("WASM stub called: {} (void) with {} params", funcName, params.length);
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
        final String funcName = name;
        final int retVal = returnValue;
        Func func = new Func(store, new FuncType(paramTypes, new Type[]{Type.I32}),
                (caller, params, results) -> {
                    EvansComputerMod.LOGGER.trace("WASM stub called: {} -> i32({}) with {} params", funcName, retVal, params.length);
                    results[0] = Val.fromI32(retVal);
                });
        hostFunctions.add(func);
        hostFunctionMap.put(name, Extern.fromFunc(func));
    }

    /**
     * Adds a stub function that takes parameters and returns a specific f64 value.
     */
    private void addStubF64Return(String name, double returnValue, Type... paramTypes) {
        final String funcName = name;
        final double retVal = returnValue;
        Func func = new Func(store, new FuncType(paramTypes, new Type[]{Type.F64}),
                (caller, params, results) -> {
                    EvansComputerMod.LOGGER.trace("WASM stub called: {} -> f64({}) with {} params", funcName, retVal, params.length);
                    results[0] = Val.fromF64(retVal);
                });
        hostFunctions.add(func);
        hostFunctionMap.put(name, Extern.fromFunc(func));
    }

    // ==================== Date/Time Implementation ====================
    // These functions provide real time support for chrono and RustPython's time module

    /** Counter for allocating "Date object handles" */
    private int nextDateHandle = 1;

    /**
     * Date.now() - Returns current timestamp in milliseconds.
     * Signature: () -> f64
     */
    private void addDateNow(String name) {
        Func func = new Func(store, new FuncType(new Type[]{}, new Type[]{Type.F64}),
                (caller, params, results) -> {
                    double now = (double) System.currentTimeMillis();
                    EvansComputerMod.LOGGER.debug("WASM Date.now() -> {}", now);
                    results[0] = Val.fromF64(now);
                });
        hostFunctions.add(func);
        hostFunctionMap.put(name, Extern.fromFunc(func));
    }

    /**
     * new Date() - Creates a Date for current time.
     * Signature: () -> i32 (returns handle)
     */
    private void addDateNew0(String name) {
        Func func = new Func(store, new FuncType(new Type[]{}, new Type[]{Type.I32}),
                (caller, params, results) -> {
                    int handle = nextDateHandle++;
                    EvansComputerMod.LOGGER.debug("WASM new Date() -> handle {}", handle);
                    results[0] = Val.fromI32(handle);
                });
        hostFunctions.add(func);
        hostFunctionMap.put(name, Extern.fromFunc(func));
    }

    /**
     * new Date(timestamp) - Creates a Date from a timestamp.
     * Signature: (i32) -> i32 (returns handle)
     */
    private void addDateNew(String name) {
        Func func = new Func(store, new FuncType(new Type[]{Type.I32}, new Type[]{Type.I32}),
                (caller, params, results) -> {
                    int handle = nextDateHandle++;
                    EvansComputerMod.LOGGER.debug("WASM new Date(timestamp) -> handle {}", handle);
                    results[0] = Val.fromI32(handle);
                });
        hostFunctions.add(func);
        hostFunctionMap.put(name, Extern.fromFunc(func));
    }

    /**
     * Date.getTime() - Returns timestamp in milliseconds.
     * Signature: (i32) -> f64
     * Note: We don't track Date objects, so always return current time.
     */
    private void addGetTime(String name) {
        Func func = new Func(store, new FuncType(new Type[]{Type.I32}, new Type[]{Type.F64}),
                (caller, params, results) -> {
                    double now = (double) System.currentTimeMillis();
                    EvansComputerMod.LOGGER.debug("WASM Date.getTime() -> {}", now);
                    results[0] = Val.fromF64(now);
                });
        hostFunctions.add(func);
        hostFunctionMap.put(name, Extern.fromFunc(func));
    }

    /**
     * Date.getTimezoneOffset() - Returns timezone offset in minutes.
     * Signature: (i32) -> f64
     */
    private void addTimezoneOffset(String name) {
        Func func = new Func(store, new FuncType(new Type[]{Type.I32}, new Type[]{Type.F64}),
                (caller, params, results) -> {
                    // JavaScript returns offset as (UTC - local) in minutes
                    // Java returns (local - UTC) in milliseconds, so we need to negate and convert
                    int offsetMs = java.util.TimeZone.getDefault().getRawOffset();
                    double offsetMinutes = -offsetMs / 60000.0;
                    EvansComputerMod.LOGGER.debug("WASM Date.getTimezoneOffset() -> {} minutes", offsetMinutes);
                    results[0] = Val.fromF64(offsetMinutes);
                });
        hostFunctions.add(func);
        hostFunctionMap.put(name, Extern.fromFunc(func));
    }

    // ==================== Object Reference Management ====================
    // wasm-bindgen uses an externref table to track JS objects. We simulate this
    // by returning incrementing handles.

    /**
     * __wbindgen_object_clone_ref - Clone an object reference.
     * Signature: (i32) -> i32
     * Returns a new handle for the "cloned" object.
     */
    private void addObjectCloneRef(String name) {
        Func func = new Func(store, new FuncType(new Type[]{Type.I32}, new Type[]{Type.I32}),
                (caller, params, results) -> {
                    int newHandle = nextObjectHandle.getAndIncrement();
                    EvansComputerMod.LOGGER.debug("WASM object_clone_ref({}) -> {}", params[0].i32(), newHandle);
                    results[0] = Val.fromI32(newHandle);
                });
        hostFunctions.add(func);
        hostFunctionMap.put(name, Extern.fromFunc(func));
    }

    /**
     * __wbindgen_externref_table_grow - Grow the externref table.
     * Signature: (i32) -> i32
     * Returns the previous table size (we just return current handle counter).
     */
    private void addExternrefTableGrow(String name) {
        Func func = new Func(store, new FuncType(new Type[]{Type.I32}, new Type[]{Type.I32}),
                (caller, params, results) -> {
                    int delta = params[0].i32();
                    int oldSize = nextObjectHandle.get();
                    nextObjectHandle.addAndGet(delta);
                    EvansComputerMod.LOGGER.debug("WASM externref_table_grow({}) -> {} (old size)", delta, oldSize);
                    results[0] = Val.fromI32(oldSize);
                });
        hostFunctions.add(func);
        hostFunctionMap.put(name, Extern.fromFunc(func));
    }

    // ==================== Crypto/Random Implementation ====================

    /** Random number generator for crypto functions */
    private final java.security.SecureRandom secureRandom = new java.security.SecureRandom();

    /**
     * __wbg_crypto_* - Get the crypto object.
     * Signature: (i32) -> i32
     * Returns a handle to a "crypto" object (non-zero so it's not null).
     */
    private void addCryptoObject(String name) {
        Func func = new Func(store, new FuncType(new Type[]{Type.I32}, new Type[]{Type.I32}),
                (caller, params, results) -> {
                    // Return a valid handle so the caller knows crypto is available
                    int handle = nextObjectHandle.getAndIncrement();
                    EvansComputerMod.LOGGER.debug("WASM crypto object requested -> handle {}", handle);
                    results[0] = Val.fromI32(handle);
                });
        hostFunctions.add(func);
        hostFunctionMap.put(name, Extern.fromFunc(func));
    }

    /**
     * __wbg_getRandomValues_* - Fill a Uint8Array with random values.
     * Signature: (i32, i32) -> void
     * First param is the crypto object handle, second is the Uint8Array handle.
     * We need to fill the array in WASM memory with random bytes.
     */
    private void addGetRandomValues(String name) {
        Func func = new Func(store, new FuncType(new Type[]{Type.I32, Type.I32}, new Type[]{}),
                (caller, params, results) -> {
                    // The second param is a Uint8Array handle, but we don't track the actual array
                    // Instead, we'll just log this was called - the actual random is handled by __getrandom_v03_custom
                    EvansComputerMod.LOGGER.debug("WASM getRandomValues called (crypto={}, array={})", params[0].i32(), params[1].i32());
                    // The real random generation happens via __getrandom_v03_custom which we already implement
                });
        hostFunctions.add(func);
        hostFunctionMap.put(name, Extern.fromFunc(func));
    }

    /**
     * __wbg_randomFillSync_* - Node.js crypto.randomFillSync.
     * Signature: (i32, i32) -> void
     * First param is the crypto object handle, second is the buffer handle.
     */
    private void addRandomFillSync(String name) {
        Func func = new Func(store, new FuncType(new Type[]{Type.I32, Type.I32}, new Type[]{}),
                (caller, params, results) -> {
                    EvansComputerMod.LOGGER.debug("WASM randomFillSync called (crypto={}, buffer={})", params[0].i32(), params[1].i32());
                    // Similar to getRandomValues - the real random is via __getrandom_v03_custom
                });
        hostFunctions.add(func);
        hostFunctionMap.put(name, Extern.fromFunc(func));
    }

    // ==================== Uint8Array Implementation ====================

    /**
     * __wbg_new_with_length_* - Create a new Uint8Array with given length.
     * Signature: (i32) -> i32
     * Returns a handle to the new array.
     */
    private void addUint8ArrayNew(String name) {
        Func func = new Func(store, new FuncType(new Type[]{Type.I32}, new Type[]{Type.I32}),
                (caller, params, results) -> {
                    int length = params[0].i32();
                    int handle = nextObjectHandle.getAndIncrement();
                    EvansComputerMod.LOGGER.debug("WASM new Uint8Array({}) -> handle {}", length, handle);
                    results[0] = Val.fromI32(handle);
                });
        hostFunctions.add(func);
        hostFunctionMap.put(name, Extern.fromFunc(func));
    }

    /**
     * __wbg_subarray_* - Get a subarray view.
     * Signature: (i32, i32, i32) -> i32
     * Returns a handle to the subarray.
     */
    private void addUint8ArraySubarray(String name) {
        Func func = new Func(store, new FuncType(new Type[]{Type.I32, Type.I32, Type.I32}, new Type[]{Type.I32}),
                (caller, params, results) -> {
                    int handle = nextObjectHandle.getAndIncrement();
                    EvansComputerMod.LOGGER.debug("WASM Uint8Array.subarray({}, {}, {}) -> handle {}",
                            params[0].i32(), params[1].i32(), params[2].i32(), handle);
                    results[0] = Val.fromI32(handle);
                });
        hostFunctions.add(func);
        hostFunctionMap.put(name, Extern.fromFunc(func));
    }

    /**
     * __wbg_length_* - Get array length.
     * Signature: (i32) -> i32
     * Returns the length of the array.
     */
    private void addUint8ArrayLength(String name) {
        Func func = new Func(store, new FuncType(new Type[]{Type.I32}, new Type[]{Type.I32}),
                (caller, params, results) -> {
                    // We don't track actual arrays, so return a reasonable default
                    // The actual length should be tracked by the WASM code
                    EvansComputerMod.LOGGER.debug("WASM Uint8Array.length({}) -> 0", params[0].i32());
                    results[0] = Val.fromI32(0);
                });
        hostFunctions.add(func);
        hostFunctionMap.put(name, Extern.fromFunc(func));
    }

    // ==================== Type Checking Functions ====================

    /**
     * __wbg___wbindgen_is_object - Check if a handle refers to an object.
     * Signature: (i32) -> i32
     * Returns 1 (true) for non-zero handles, 0 (false) for zero (null).
     * This is critical for getrandom to detect the crypto object.
     */
    private void addIsObject(String name) {
        Func func = new Func(store, new FuncType(new Type[]{Type.I32}, new Type[]{Type.I32}),
                (caller, params, results) -> {
                    int handle = params[0].i32();
                    // Non-zero handles are valid objects
                    int result = (handle != 0) ? 1 : 0;
                    EvansComputerMod.LOGGER.debug("WASM is_object({}) -> {}", handle, result);
                    results[0] = Val.fromI32(result);
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
            EvansComputerMod.LOGGER.error("Error in hostGetrandom", e);
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

            EvansComputerMod.LOGGER.debug("Module requires import: {}::{}", moduleName, name);

            // Look up the host function by name
            if (hostFunctionMap.containsKey(name)) {
                imports.add(hostFunctionMap.get(name));
                EvansComputerMod.LOGGER.debug("  -> Matched to host function: {}", name);
            } else {
                // Unknown import - this will cause instantiation to fail
                // Log a warning so we know what's missing
                EvansComputerMod.LOGGER.warn("Unknown WASM import: {}::{} (type: {})",
                        moduleName, name, importType.type());

                // Try to provide a stub based on the import type
                Extern stub = createStubImport(importType);
                if (stub != null) {
                    imports.add(stub);
                    EvansComputerMod.LOGGER.debug("  -> Created stub for: {}", name);
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
        // Check for interrupt - this is a frequently called function
        checkInterrupted();

        if (memory == null) {
            EvansComputerMod.LOGGER.error("WASM memory not initialized");
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
            writeToTerminal(text);

            EvansComputerMod.LOGGER.debug("WASM wrote to terminal: {}", text);
            return len;

        } catch (WasmInterruptedException e) {
            // Re-throw interrupt exceptions - don't swallow them!
            throw e;
        } catch (Exception e) {
            EvansComputerMod.LOGGER.error("Error reading from WASM memory", e);
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

    /** Maximum path depth to prevent resource exhaustion. */
    private static final int MAX_PATH_DEPTH = 10;
    /** Maximum total path length. */
    private static final int MAX_PATH_LENGTH = 256;

    private Path sanitizePath(String filename) {
        if (filename == null || filename.isEmpty()) {
            return null;
        }
        if (filename.length() > MAX_PATH_LENGTH) {
            EvansComputerMod.LOGGER.warn("Rejected path exceeding max length: {}", filename.length());
            return null;
        }
        if (filename.contains("..") || filename.startsWith("/") || filename.startsWith("\\") ||
            filename.contains("\\") || filename.contains(":") || filename.contains("\0")) {
            EvansComputerMod.LOGGER.warn("Rejected unsafe file path: {}", filename);
            return null;
        }
        long depth = filename.chars().filter(c -> c == '/').count();
        if (depth > MAX_PATH_DEPTH) {
            EvansComputerMod.LOGGER.warn("Rejected path exceeding max depth: {}", filename);
            return null;
        }
        Path resolved = computerStoragePath.resolve(filename).normalize();
        if (!resolved.startsWith(computerStoragePath)) {
            EvansComputerMod.LOGGER.warn("Path escaped storage directory: {}", filename);
            return null;
        }
        return resolved;
    }

    /**
     * Host function: writes data to a file.
     */
    private int hostFileWrite(int pathPtr, int pathLen, int dataPtr, int dataLen) {
        checkInterrupted();

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

            // Create parent directories if needed (for nested paths like "dir/file.txt")
            Path parent = filePath.getParent();
            if (parent != null && !Files.exists(parent)) {
                Files.createDirectories(parent);
            }

            Files.write(filePath, data, StandardOpenOption.CREATE, StandardOpenOption.TRUNCATE_EXISTING);
            EvansComputerMod.LOGGER.debug("Wrote {} bytes to file: {}", dataLen, filename);
            return dataLen;
        } catch (Exception e) {
            EvansComputerMod.LOGGER.error("Error writing file: {}", filename, e);
            return -1;
        }
    }

    /**
     * Host function: reads data from a file.
     */
    private int hostFileRead(int pathPtr, int pathLen, int bufPtr, int bufLen) {
        checkInterrupted();

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

            EvansComputerMod.LOGGER.debug("Read {} bytes from file: {}", bytesToRead, filename);
            return bytesToRead;
        } catch (Exception e) {
            EvansComputerMod.LOGGER.error("Error reading file: {}", filename, e);
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
            EvansComputerMod.LOGGER.debug("Deleted file: {}", filename);
            return 1;
        } catch (Exception e) {
            EvansComputerMod.LOGGER.error("Error deleting file: {}", filename, e);
            return 0;
        }
    }

    private int hostFileMkdir(int pathPtr, int pathLen) {
        String dirname = readStringFromMemory(pathPtr, pathLen);
        Path dirPath = sanitizePath(dirname);
        if (dirPath == null) {
            return -1;
        }
        try {
            Files.createDirectories(dirPath);
            EvansComputerMod.LOGGER.debug("Created directory: {}", dirname);
            return 0;
        } catch (Exception e) {
            EvansComputerMod.LOGGER.error("Error creating directory: {}", dirname, e);
            return -1;
        }
    }

    private int hostFileIsDir(int pathPtr, int pathLen) {
        String filename = readStringFromMemory(pathPtr, pathLen);
        if (filename == null || filename.isEmpty()) {
            return 1; // Empty path = root, which is a directory
        }
        Path filePath = sanitizePath(filename);
        if (filePath == null) {
            return 0;
        }
        return Files.isDirectory(filePath) ? 1 : 0;
    }

    private int hostFileListDir(int pathPtr, int pathLen, int bufPtr, int bufLen) {
        if (memory == null) {
            return -1;
        }
        String dirname = readStringFromMemory(pathPtr, pathLen);
        Path dirPath;
        if (dirname == null || dirname.isEmpty()) {
            dirPath = computerStoragePath;
        } else {
            dirPath = sanitizePath(dirname);
            if (dirPath == null) {
                return -1;
            }
        }
        if (!Files.isDirectory(dirPath)) {
            return -1;
        }
        try {
            String listing = Files.list(dirPath)
                    .map(p -> {
                        String prefix = Files.isDirectory(p) ? "d:" : "f:";
                        return prefix + p.getFileName().toString();
                    })
                    .sorted()
                    .collect(Collectors.joining("\n"));
            return writeStringToMemory(listing, bufPtr, bufLen);
        } catch (Exception e) {
            EvansComputerMod.LOGGER.error("Error listing directory: {}", dirname, e);
            return -1;
        }
    }

    /**
     * Converts a relative side index to an absolute Minecraft Direction.
     * Relative sides are based on the host's facing direction.
     *
     * @param relativeSide 0=DOWN, 1=UP, 2=FRONT, 3=BACK, 4=LEFT, 5=RIGHT
     * @return The absolute Direction
     */
    private Direction relativeToAbsolute(int relativeSide) {
        IRedstoneProvider provider = host.getRedstoneProvider();
        if (provider != null) {
            return provider.relativeToAbsolute(relativeSide);
        }
        // Fallback: assume NORTH facing
        Direction facing = Direction.NORTH;
        return switch (relativeSide) {
            case 0 -> Direction.DOWN;
            case 1 -> Direction.UP;
            case 2 -> facing;                      // FRONT - the direction the screen faces
            case 3 -> facing.getOpposite();        // BACK - opposite of front
            case 4 -> facing.getCounterClockWise(); // LEFT - to the left of the terminal
            case 5 -> facing.getClockWise();       // RIGHT - to the right of the terminal
            default -> Direction.NORTH;
        };
    }

    /**
     * Host function: sets redstone output power for a specific side.
     * @param relativeSide The relative side index (0=DOWN, 1=UP, 2=FRONT, 3=BACK, 4=LEFT, 5=RIGHT)
     * @param power The power level (0-15)
     * @return 0 on success, -1 on failure
     */
    private int hostRedstoneSetOutput(int relativeSide, int power) {
        if (relativeSide < 0 || relativeSide > 5 || power < 0 || power > 15) {
            return -1;
        }

        IRedstoneProvider provider = host.getRedstoneProvider();
        if (provider == null) {
            return -1;
        }

        try {
            // Convert relative side to absolute direction
            Direction absoluteDir = provider.relativeToAbsolute(relativeSide);
            int absoluteSide = absoluteDir.ordinal();

            // Schedule the redstone update on the main server thread
            if (host.getServer() != null) {
                host.getServer().execute(() -> {
                    provider.setRedstoneOutput(absoluteSide, power);
                });
            } else {
                // Fallback: direct call (might be on main thread already)
                provider.setRedstoneOutput(absoluteSide, power);
            }
            return 0;
        } catch (Exception e) {
            EvansComputerMod.LOGGER.error("Error setting redstone output", e);
            return -1;
        }
    }

    /**
     * Host function: reads redstone input power for a specific relative side.
     * @param relativeSide The relative side index (0=DOWN, 1=UP, 2=FRONT, 3=BACK, 4=LEFT, 5=RIGHT)
     * @return The power level (0-15), or -1 on invalid side
     */
    private int hostRedstoneGetInput(int relativeSide) {
        if (relativeSide < 0 || relativeSide > 5) {
            return -1;
        }

        IRedstoneProvider provider = host.getRedstoneProvider();
        if (provider == null) {
            return 0;
        }

        try {
            Direction absoluteDir = provider.relativeToAbsolute(relativeSide);
            return provider.getRedstoneInput(absoluteDir.ordinal());
        } catch (Exception e) {
            EvansComputerMod.LOGGER.error("Error reading redstone input", e);
            return -1;
        }
    }

    /**
     * Host function: reads all 6 redstone input levels into a WASM memory buffer.
     * Writes 6 little-endian i32 values (24 bytes) at buf_ptr, in relative side order.
     * @param bufPtr WASM memory address to write to
     * @return 0 on success, -1 on failure
     */
    private int hostRedstoneGetAllInput(int bufPtr) {
        if (memory == null) return -1;

        IRedstoneProvider provider = host.getRedstoneProvider();
        if (provider == null) {
            return -1;
        }

        try {
            ByteBuffer buffer = memory.buffer(store);
            buffer.position(bufPtr);
            for (int relativeSide = 0; relativeSide < 6; relativeSide++) {
                Direction absoluteDir = provider.relativeToAbsolute(relativeSide);
                int power = provider.getRedstoneInput(absoluteDir.ordinal());
                // Write as little-endian i32
                buffer.put((byte) (power & 0xFF));
                buffer.put((byte) ((power >> 8) & 0xFF));
                buffer.put((byte) ((power >> 16) & 0xFF));
                buffer.put((byte) ((power >> 24) & 0xFF));
            }
            return 0;
        } catch (Exception e) {
            EvansComputerMod.LOGGER.error("Error reading all redstone inputs", e);
            return -1;
        }
    }

    /**
     * Host function: polls for the next pending interrupt.
     * Writes the payload into WASM memory at bufPtr (up to bufLen bytes).
     * Returns the IRQ number (>= 0) if an interrupt was polled, or -1 if none pending.
     * This is a pull-based API that avoids WASM reentrancy.
     */
    private int hostInterruptPoll(int bufPtr, int bufLen) {
        checkInterrupted();
        InterruptEvent evt = interruptQueue.poll();
        if (evt == null) {
            lastInterruptPayloadLen = 0;
            return -1;
        }

        // Write payload to WASM memory
        if (memory != null) {
            byte[] payloadBytes = evt.payload.getBytes(StandardCharsets.UTF_8);
            int writeLen = Math.min(payloadBytes.length, bufLen);
            ByteBuffer buffer = memory.buffer(store);
            buffer.position(bufPtr);
            buffer.put(payloadBytes, 0, writeLen);
            lastInterruptPayloadLen = writeLen;
        } else {
            lastInterruptPayloadLen = 0;
        }

        return evt.irq;
    }

    /**
     * Host function: sleeps for the specified number of milliseconds.
     * This blocks the WASM execution but not the game server (since WASM runs on a background thread).
     * The Rust side handles chunking sleeps for interrupt delivery.
     * @param milliseconds Time to sleep (clamped to 0-60000ms)
     */
    private void hostSleepMs(int milliseconds) {
        checkInterrupted();  // Check before sleeping
        // Clamp to reasonable range (0 to 60 seconds max)
        int clampedMs = Math.max(0, Math.min(60000, milliseconds));
        try {
            Thread.sleep(clampedMs);
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
            throw new WasmInterruptedException("Sleep interrupted");
        }
        checkInterrupted();  // Check after sleeping
    }

    /**
     * Host function: reads a line of text input from the user.
     * Displays the prompt, then blocks while consuming keystrokes from inputQueue
     * until Enter is pressed. Echoes characters and handles backspace.
     *
     * @param promptPtr WASM memory address of prompt string (already displayed by Rust side)
     * @param promptLen Length of prompt string (unused -- Rust displays it)
     * @param bufPtr    WASM memory address to write the result line
     * @param bufLen    Maximum bytes to write
     * @return Number of bytes written, or -1 on error
     */
    private int hostReadLine(int promptPtr, int promptLen, int bufPtr, int bufLen) {
        if (interrupted) return -2; // Interrupted before we started

        StringBuilder lineBuffer = new StringBuilder();

        while (!shutdownRequested) {
            if (interrupted) return -2; // Interrupted -- return gracefully, let Rust handle reset

            try {
                String input = inputQueue.poll(100, TimeUnit.MILLISECONDS);
                if (input == null) continue;

                for (int i = 0; i < input.length(); i++) {
                    char c = input.charAt(i);

                    if (c == '\n' || c == '\r') {
                        // Enter pressed -- echo newline and return the line
                        writeToTerminal("\n");
                        syncTerminalToClients();

                        // Write result to WASM memory
                        byte[] resultBytes = lineBuffer.toString().getBytes(StandardCharsets.UTF_8);
                        int writeLen = Math.min(resultBytes.length, bufLen);
                        if (memory != null && writeLen > 0) {
                            ByteBuffer buffer = memory.buffer(store);
                            buffer.position(bufPtr);
                            buffer.put(resultBytes, 0, writeLen);
                        }
                        return writeLen;
                    } else if (c == 0x14) {
                        // Ctrl+T -- return interrupt code, let Rust side handle reset
                        return -2;
                    } else if (c == 8 || c == 127) {
                        // Backspace
                        if (lineBuffer.length() > 0) {
                            lineBuffer.deleteCharAt(lineBuffer.length() - 1);
                            writeToTerminal("\b \b");
                        }
                    } else if (c >= 32) {
                        // Printable character
                        if (lineBuffer.length() < bufLen) {
                            lineBuffer.append(c);
                            writeToTerminal(String.valueOf(c));
                        }
                    }
                    // Ignore other control characters
                }

                syncTerminalToClients();
            } catch (InterruptedException e) {
                // Thread was interrupted (by wasmHost.interrupt()) -- return gracefully
                Thread.interrupted(); // Clear the flag
                return -2;
            }
        }

        return -1; // Shutdown requested
    }

    // === CC:Tweaked Peripheral Host Functions ===

    // Peripheral manager and invoker (lazily initialized)
    private PeripheralManager peripheralManager;
    private PeripheralMethodInvoker peripheralInvoker;

    // Module method invoker for annotation-driven auto-registration
    private ModuleMethodInvoker moduleMethodInvoker;

    /**
     * Gets or creates the peripheral manager for this computer.
     */
    private PeripheralManager getPeripheralManager() {
        IWorldAccess worldAccess = host.getWorldAccess();
        if (peripheralManager == null && worldAccess != null) {
            peripheralManager = new PeripheralManager(worldAccess);
            peripheralManager.scanPeripherals();
        }
        return peripheralManager;
    }

    /**
     * Gets or creates the peripheral invoker.
     */
    private PeripheralMethodInvoker getPeripheralInvoker() {
        if (peripheralInvoker == null) {
            peripheralInvoker = new PeripheralMethodInvoker();
        }
        return peripheralInvoker;
    }

    /**
     * Rescans peripherals. Called when neighbors change or after world reload.
     * This will create the peripheral manager if it doesn't exist yet.
     */
    public void rescanPeripherals() {
        PeripheralManager pm = getPeripheralManager();
        if (pm != null) {
            pm.scanPeripherals();
        }
    }

    /**
     * Host function: lists all connected peripherals as JSON.
     * Returns bytes written to buffer, or -1 on error.
     */
    private int hostPeripheralList(int bufPtr, int bufLen) {
        checkInterrupted();

        if (memory == null) {
            return -1;
        }

        PeripheralManager pm = getPeripheralManager();
        if (pm == null || !PeripheralManager.isCCAvailable()) {
            // Return empty array if CC is not available
            String json = "[]";
            return writeStringToMemory(json, bufPtr, bufLen);
        }

        String json = pm.listPeripheralsAsJson();
        return writeStringToMemory(json, bufPtr, bufLen);
    }

    /**
     * Host function: gets method names for a peripheral.
     * Returns bytes written to buffer, or -1 on error.
     */
    private int hostPeripheralGetMethods(int namePtr, int nameLen, int bufPtr, int bufLen) {
        checkInterrupted();

        if (memory == null) {
            return -1;
        }

        String peripheralName = readStringFromMemory(namePtr, nameLen);
        if (peripheralName == null) {
            return writeStringToMemory("{\"ok\":false,\"error\":\"Invalid peripheral name\"}", bufPtr, bufLen);
        }

        PeripheralManager pm = getPeripheralManager();
        if (pm == null || !PeripheralManager.isCCAvailable()) {
            return writeStringToMemory("{\"ok\":false,\"error\":\"CC:Tweaked not available\"}", bufPtr, bufLen);
        }

        String json = pm.getMethodNamesAsJson(peripheralName);
        return writeStringToMemory(json, bufPtr, bufLen);
    }

    /**
     * Host function: calls a peripheral method with JSON arguments.
     * Returns bytes written to result buffer, or -1 on error.
     */
    private int hostPeripheralCall(int namePtr, int nameLen, int methodPtr, int methodLen,
                                    int argsPtr, int argsLen, int resultPtr, int resultLen) {
        checkInterrupted();

        if (memory == null) {
            return -1;
        }

        String peripheralName = readStringFromMemory(namePtr, nameLen);
        String methodName = readStringFromMemory(methodPtr, methodLen);
        String argsJson = argsLen > 0 ? readStringFromMemory(argsPtr, argsLen) : "[]";

        if (peripheralName == null || methodName == null) {
            return writeStringToMemory("{\"ok\":false,\"error\":\"Invalid arguments\"}", resultPtr, resultLen);
        }

        PeripheralManager pm = getPeripheralManager();
        if (pm == null || !PeripheralManager.isCCAvailable()) {
            return writeStringToMemory("{\"ok\":false,\"error\":\"CC:Tweaked not available\"}", resultPtr, resultLen);
        }

        // Find the peripheral
        var peripheralOpt = pm.getPeripheral(peripheralName);
        if (peripheralOpt.isEmpty()) {
            return writeStringToMemory("{\"ok\":false,\"error\":\"Peripheral not found: " + peripheralName + "\"}", resultPtr, resultLen);
        }

        // Call the method
        PeripheralMethodInvoker invoker = getPeripheralInvoker();
        var server = host.getServer();
        String resultJson = invoker.invokeMethod(peripheralOpt.get().getPeripheral(), methodName, argsJson, server);

        return writeStringToMemory(resultJson, resultPtr, resultLen);
    }

    /**
     * Writes a string to WASM memory. Returns bytes written or -1 on error.
     */
    private int writeStringToMemory(String str, int bufPtr, int bufLen) {
        if (memory == null || str == null) {
            return -1;
        }

        try {
            byte[] bytes = str.getBytes(StandardCharsets.UTF_8);
            int bytesToWrite = Math.min(bytes.length, bufLen);

            ByteBuffer buffer = memory.buffer(store);
            buffer.position(bufPtr);
            buffer.put(bytes, 0, bytesToWrite);

            return bytesToWrite;
        } catch (Exception e) {
            EvansComputerMod.LOGGER.error("Error writing to WASM memory", e);
            return -1;
        }
    }

    // ==================== Module Call Bridge ====================

    /**
     * Gets or creates the module method invoker.
     */
    private ModuleMethodInvoker getModuleMethodInvoker() {
        if (moduleMethodInvoker == null) {
            moduleMethodInvoker = new ModuleMethodInvoker();
        }
        return moduleMethodInvoker;
    }

    /**
     * Host function: calls a registered module method using binary protocol.
     * Args and result are binary-encoded (not JSON).
     */
    private int hostModuleCall(int modulePtr, int moduleLen, int methodPtr, int methodLen,
                               int argsPtr, int argsLen, int resultPtr, int resultLen) {
        checkInterrupted();

        if (memory == null) {
            return -1;
        }

        String moduleName = readStringFromMemory(modulePtr, moduleLen);
        String methodName = readStringFromMemory(methodPtr, methodLen);

        if (moduleName == null || methodName == null) {
            byte[] errorResult = ModuleMethodInvoker.serializeError("Invalid arguments");
            return writeBytesToMemory(errorResult, resultPtr, resultLen);
        }

        // Read args as raw binary (no string conversion)
        byte[] argsBinary = readBytesFromMemory(argsPtr, argsLen);

        ModuleMethodInvoker invoker = getModuleMethodInvoker();
        byte[] resultBinary = invoker.invokeMethod(host, moduleName, methodName, argsBinary);

        return writeBytesToMemory(resultBinary, resultPtr, resultLen);
    }

    /**
     * Reads raw bytes from WASM memory.
     */
    private byte[] readBytesFromMemory(int ptr, int len) {
        if (memory == null || len <= 0) {
            return new byte[0];
        }
        try {
            ByteBuffer buffer = memory.buffer(store);
            byte[] bytes = new byte[len];
            buffer.position(ptr);
            buffer.get(bytes, 0, len);
            return bytes;
        } catch (Exception e) {
            return new byte[0];
        }
    }

    /**
     * Writes raw bytes to WASM memory. Returns bytes written or -1 on error.
     */
    private int writeBytesToMemory(byte[] data, int ptr, int maxLen) {
        if (memory == null || data == null) {
            return -1;
        }
        try {
            int bytesToWrite = Math.min(data.length, maxLen);
            ByteBuffer buffer = memory.buffer(store);
            buffer.position(ptr);
            buffer.put(data, 0, bytesToWrite);
            return bytesToWrite;
        } catch (Exception e) {
            EvansComputerMod.LOGGER.error("Error writing bytes to WASM memory", e);
            return -1;
        }
    }

    /**
     * Host function: returns JSON metadata of all registered modules.
     */
    private int hostModuleList(int bufPtr, int bufLen) {
        checkInterrupted();

        if (memory == null) {
            return -1;
        }

        String json = ComputerModuleRegistry.getMetadataJson();
        return writeStringToMemory(json, bufPtr, bufLen);
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
            EvansComputerMod.LOGGER.error("Error listing files", e);
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

            EvansComputerMod.LOGGER.info("Loading WASM module: {}", fileName);

            // Create imports in the order required by the module
            List<Extern> imports = createImportsForModule(module);

            EvansComputerMod.LOGGER.info("Providing {} imports to WASM module", imports.size());

            // Create instance with imports
            instance = new Instance(store, module, imports);

            // Get the memory export for reading strings
            Optional<Memory> memoryOpt = instance.getMemory(store, "memory");
            if (memoryOpt.isPresent()) {
                memory = memoryOpt.get();
                EvansComputerMod.LOGGER.debug("Got WASM memory export");
            } else {
                EvansComputerMod.LOGGER.warn("WASM module does not export 'memory'");
            }

            EvansComputerMod.LOGGER.info("Successfully loaded WASM module: {}", fileName);

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
     * This must be called from the worker thread or before the worker thread is started.
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
                    EvansComputerMod.LOGGER.info("Executed WASM entry point: {}", entryPoint);
                    return;
                } catch (WasmtimeException e) {
                    throw new WasmManager.WasmExecutionException(
                            "Error executing " + entryPoint + ": " + e.getMessage(), e);
                }
            }
        }

        EvansComputerMod.LOGGER.warn("No entry point found in WASM module");
    }

    /**
     * Clears the interrupt flag, allowing WASM execution to resume.
     * Called after the OS has reset to shell mode following a Ctrl+T interrupt.
     */
    public void clearInterrupt() {
        interrupted = false;
        EvansComputerMod.LOGGER.debug("WASM interrupt flag cleared");
    }

    /**
     * Checks if the WASM module has faulted and should not be used.
     */
    public boolean isFaulted() {
        return faulted;
    }

    /**
     * Checks if the WASM execution has been interrupted.
     */
    public boolean isInterrupted() {
        return interrupted;
    }

    /**
     * Signals the WASM module to interrupt execution.
     * This sets the interrupt flag which is checked by host functions.
     * When a host function sees the interrupt flag, it throws an InterruptedException
     * to abort WASM execution.
     */
    public void interrupt() {
        interrupted = true;
        EvansComputerMod.LOGGER.info("WASM execution interrupt requested");
        // Also interrupt the worker thread in case it's blocked (e.g., in Thread.sleep())
        if (workerThread != null && workerThread.isAlive()) {
            workerThread.interrupt();
        }
    }

    /**
     * Checks if execution should be interrupted and throws if so.
     * Called by host functions to allow interruption of long-running WASM code.
     */
    private void checkInterrupted() {
        if (interrupted) {
            EvansComputerMod.LOGGER.info("checkInterrupted() throwing WasmInterruptedException");
            throw new WasmInterruptedException("WASM execution interrupted");
        }
    }

    /**
     * Exception thrown when WASM execution is interrupted.
     */
    public static class WasmInterruptedException extends RuntimeException {
        public WasmInterruptedException(String message) {
            super(message);
        }
    }

    /**
     * Queues input to be processed by the WASM worker thread.
     * This is called when the user enters input in the terminal.
     * The input is processed asynchronously to keep the main server thread responsive.
     *
     * @param line The input line from the user
     */
    public void sendInput(String line) {
        if (instance == null || faulted) {
            if (faulted) {
                writeToTerminal("\n[WASM faulted - close and reopen terminal to reset]\n");
            }
            return;
        }

        // Queue the input for the worker thread
        try {
            inputQueue.put(line);
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
            EvansComputerMod.LOGGER.warn("Interrupted while queuing input");
        }
    }

    /** Get the primary network MAC address (interface 0). */
    public byte[] getNetworkMac() {
        return networkMacs[0];
    }

    /** Get all network MAC addresses. */
    public byte[][] getNetworkMacs() {
        return networkMacs;
    }

    @Override
    public void close() {
        // Unregister all NICs from network hub
        NetworkHub hub = NetworkHub.getInstance();
        if (hub != null && networkMacs != null) {
            for (byte[] mac : networkMacs) {
                hub.unregisterNic(mac);
            }
        }

        // Signal worker thread to stop
        shutdownRequested = true;
        interrupted = true;  // Also set interrupt to abort any running WASM

        // Interrupt and wait for worker thread to finish
        if (workerThread != null && workerThread.isAlive()) {
            workerThread.interrupt();
            try {
                workerThread.join(1000);  // Wait up to 1 second
            } catch (InterruptedException e) {
                Thread.currentThread().interrupt();
            }
            if (workerThread.isAlive()) {
                EvansComputerMod.LOGGER.warn("WASM worker thread did not terminate in time");
            }
        }

        // Clear input queue
        inputQueue.clear();

        // Close WASM resources
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
