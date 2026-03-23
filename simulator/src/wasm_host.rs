//! Wasmtime engine, Linker, module loading, and import resolution.

use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::Receiver;
use std::sync::Arc;

use wasmtime::*;

use crate::filesystem::FileSystem;
use crate::host_functions;
use crate::interrupts::InterruptQueue;
use crate::redstone::RedstoneState;
use crate::terminal_io::TerminalBuffer;
use crate::wasm_bindgen_stubs;

/// Memory addresses matching the Java TerminalWasmHost
const INPUT_BUFFER_ADDR: i32 = 0x10000;
const _INTERRUPT_BUFFER_ADDR: i32 = 0x11000;

/// State shared with WASM host functions via wasmtime's Store.
pub struct HostState {
    pub terminal: TerminalBuffer,
    pub filesystem: FileSystem,
    pub redstone: RedstoneState,
    pub interrupt_queue: InterruptQueue,
    pub input_rx: Arc<ChannelReceiver>,
    pub shutdown: Arc<AtomicBool>,
    pub last_interrupt_payload_len: i32,
    pub next_object_handle: i32,
}

impl HostState {
    /// Create a dummy state for probe operations. Not for actual use.
    pub fn dummy() -> Self {
        let (_tx, rx) = std::sync::mpsc::channel();
        Self {
            terminal: TerminalBuffer::new(80, 24),
            filesystem: FileSystem::new("/tmp/dummy-sim".into()),
            redstone: RedstoneState::new(),
            interrupt_queue: InterruptQueue::new(),
            input_rx: Arc::new(ChannelReceiver(std::sync::Mutex::new(rx))),
            shutdown: Arc::new(AtomicBool::new(false)),
            last_interrupt_payload_len: 0,
            next_object_handle: 1,
        }
    }
}

/// Wrapper to make mpsc::Receiver shareable via Arc
pub struct ChannelReceiver(pub std::sync::Mutex<Receiver<String>>);

impl ChannelReceiver {
    pub fn recv_timeout(&self, timeout: std::time::Duration) -> Result<String, std::sync::mpsc::RecvTimeoutError> {
        self.0.lock().unwrap().recv_timeout(timeout)
    }
}

/// The WASM host that manages the engine, store, and instance.
pub struct WasmHost {
    store: Store<HostState>,
    instance: Instance,
}

impl WasmHost {
    /// Load and instantiate the WASM module.
    pub fn new(
        wasm_path: &Path,
        terminal: TerminalBuffer,
        filesystem: FileSystem,
        redstone: RedstoneState,
        interrupt_queue: InterruptQueue,
        input_rx: Receiver<String>,
        shutdown: Arc<AtomicBool>,
    ) -> Result<Self> {
        let engine = Engine::default();
        let module = Module::from_file(&engine, wasm_path)?;

        let state = HostState {
            terminal,
            filesystem,
            redstone,
            interrupt_queue,
            input_rx: Arc::new(ChannelReceiver(std::sync::Mutex::new(input_rx))),
            shutdown,
            last_interrupt_payload_len: 0,
            next_object_handle: 1,
        };

        let mut store = Store::new(&engine, state);
        let mut linker = Linker::new(&engine);

        // Register known host functions first
        host_functions::register_all(&mut linker)?;

        // Register wasm-bindgen stubs for all remaining imports
        wasm_bindgen_stubs::register_all_stubs(
            &mut linker,
            &module,
            host_functions::KNOWN_FUNCTION_NAMES,
        )?;

        let instance = linker.instantiate(&mut store, &module)?;

        Ok(Self { store, instance })
    }

    /// Call the WASM main() function.
    pub fn call_main(&mut self) -> Result<()> {
        let main_fn = self.instance
            .get_typed_func::<(), ()>(&mut self.store, "main")?;
        main_fn.call(&mut self.store, ())?;
        self.store.data_mut().terminal.render()?;
        Ok(())
    }

    /// Send input to the WASM module by writing to the input buffer and calling on_input.
    pub fn send_input(&mut self, input: &str) -> Result<()> {
        let memory = self.instance
            .get_memory(&mut self.store, "memory")
            .ok_or_else(|| anyhow::anyhow!("No memory export found"))?;

        let bytes = input.as_bytes();
        let data = memory.data_mut(&mut self.store);
        let start = INPUT_BUFFER_ADDR as usize;
        let end = start + bytes.len();
        if end <= data.len() {
            data[start..end].copy_from_slice(bytes);
        }

        // Call on_input(ptr, len)
        let on_input = self.instance
            .get_typed_func::<(i32, i32), ()>(&mut self.store, "on_input")?;
        match on_input.call(&mut self.store, (INPUT_BUFFER_ADDR, bytes.len() as i32)) {
            Ok(()) => {},
            Err(e) => {
                // Check if this is an interrupt-related error
                let msg = format!("{}", e);
                if msg.contains("interrupt") || msg.contains("Interrupt") {
                    // Interrupted - clear and continue
                } else {
                    return Err(e);
                }
            }
        }

        self.store.data_mut().terminal.render()?;
        Ok(())
    }

    /// Deliver pending interrupts to the WASM module via on_interrupt export.
    pub fn deliver_interrupts(&mut self) -> Result<()> {
        let interrupt_queue = self.store.data().interrupt_queue.clone();

        while let Some(evt) = interrupt_queue.pop() {
            let memory = self.instance
                .get_memory(&mut self.store, "memory")
                .ok_or_else(|| anyhow::anyhow!("No memory export found"))?;

            let payload_bytes = evt.payload.as_bytes();
            let buf_addr = _INTERRUPT_BUFFER_ADDR as usize;
            let data = memory.data_mut(&mut self.store);
            let end = buf_addr + payload_bytes.len();
            if end <= data.len() {
                data[buf_addr..end].copy_from_slice(payload_bytes);
            }

            // Call on_interrupt(irq, ptr, len)
            if let Ok(on_interrupt) = self.instance
                .get_typed_func::<(i32, i32, i32), ()>(&mut self.store, "on_interrupt")
            {
                match on_interrupt.call(
                    &mut self.store,
                    (evt.irq, _INTERRUPT_BUFFER_ADDR, payload_bytes.len() as i32),
                ) {
                    Ok(()) => {},
                    Err(e) => {
                        let msg = format!("{}", e);
                        if !msg.contains("interrupt") && !msg.contains("Interrupt") {
                            eprintln!("[Simulator] Error delivering interrupt IRQ={}: {}", evt.irq, e);
                        }
                    }
                }
            }
        }

        self.store.data_mut().terminal.render()?;
        Ok(())
    }

    /// Check if shutdown was requested.
    pub fn is_shutdown(&self) -> bool {
        self.store.data().shutdown.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Get a reference to the input receiver for the worker loop.
    pub fn input_rx(&self) -> Arc<ChannelReceiver> {
        self.store.data().input_rx.clone()
    }

    /// Run the worker loop: drain interrupts, poll input, call on_input.
    pub fn worker_loop(&mut self) {
        let input_rx = self.input_rx();
        let shutdown = self.store.data().shutdown.clone();

        while !shutdown.load(std::sync::atomic::Ordering::Relaxed) {
            // Drain pending interrupts
            if let Err(e) = self.deliver_interrupts() {
                eprintln!("[Simulator] Error delivering interrupts: {}", e);
            }

            // Poll input with timeout
            match input_rx.recv_timeout(std::time::Duration::from_millis(100)) {
                Ok(input) => {
                    if let Err(e) = self.send_input(&input) {
                        eprintln!("[Simulator] Error processing input: {}", e);
                    }
                    // Drain interrupts again after input
                    if let Err(e) = self.deliver_interrupts() {
                        eprintln!("[Simulator] Error delivering interrupts: {}", e);
                    }
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
    }
}
