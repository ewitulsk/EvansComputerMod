//! Wasmtime engine, Linker, module loading, and import resolution.

use std::any::Any;
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::Receiver;
use std::sync::Arc;

use wasmtime::*;

use crate::filesystem::FileSystem;
use crate::host;
use crate::interrupts::InterruptQueue;
use crate::network::NetworkState;
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
    /// Custom state storage for extension host functions.
    ///
    /// Keyed by `TypeId` so each extension type gets its own slot.
    /// Use `insert_custom::<T>()`, `get_custom::<T>()`, and
    /// `get_custom_mut::<T>()` to access.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// // Define your state type
    /// struct MyPeripheralState { counter: u32 }
    ///
    /// // Store it during setup
    /// caller.data_mut().insert_custom(MyPeripheralState { counter: 0 });
    ///
    /// // Read it in a host function
    /// let state = caller.data().get_custom::<MyPeripheralState>().unwrap();
    /// ```
    pub custom: HashMap<std::any::TypeId, Box<dyn Any + Send>>,
}

impl HostState {
    /// Store a custom state value, keyed by its concrete type.
    pub fn insert_custom<T: Any + Send>(&mut self, value: T) {
        self.custom.insert(std::any::TypeId::of::<T>(), Box::new(value));
    }

    /// Get a reference to a custom state value by type.
    pub fn get_custom<T: Any + Send>(&self) -> Option<&T> {
        self.custom
            .get(&std::any::TypeId::of::<T>())
            .and_then(|b| b.downcast_ref())
    }

    /// Get a mutable reference to a custom state value by type.
    pub fn get_custom_mut<T: Any + Send>(&mut self) -> Option<&mut T> {
        self.custom
            .get_mut(&std::any::TypeId::of::<T>())
            .and_then(|b| b.downcast_mut())
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
        network: Option<NetworkState>,
    ) -> Result<Self> {
        let engine = Engine::default();
        let module = Module::from_file(&engine, wasm_path)?;

        let mut state = HostState {
            terminal,
            filesystem,
            redstone,
            interrupt_queue,
            input_rx: Arc::new(ChannelReceiver(std::sync::Mutex::new(input_rx))),
            shutdown,
            last_interrupt_payload_len: 0,
            next_object_handle: 1,
            custom: HashMap::new(),
        };

        // Insert network state if provided
        if let Some(net) = network {
            state.insert_custom(net);
        }

        let mut store = Store::new(&engine, state);
        let mut linker = Linker::new(&engine);

        // Register known host functions first
        host::register_all(&mut linker)?;

        // Register wasm-bindgen stubs for all remaining imports
        let known = host::known_names();
        let known_refs: Vec<&str> = known.iter().copied().collect();
        wasm_bindgen_stubs::register_all_stubs(
            &mut linker,
            &module,
            &known_refs,
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
