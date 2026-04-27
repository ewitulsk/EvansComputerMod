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
use crate::terminal_io::FramebufferRenderer;
use crate::wasm_bindgen_stubs;

/// Memory addresses matching the Java TerminalWasmHost
const INPUT_BUFFER_ADDR: i32 = 0x10000;
const _INTERRUPT_BUFFER_ADDR: i32 = 0x11000;

/// State shared with WASM host functions via wasmtime's Store.
pub struct HostState {
    pub renderer: FramebufferRenderer,
    pub filesystem: FileSystem,
    pub redstone: RedstoneState,
    pub interrupt_queue: InterruptQueue,
    pub input_rx: Arc<ChannelReceiver>,
    pub shutdown: Arc<AtomicBool>,
    pub last_interrupt_payload_len: i32,
    pub next_object_handle: i32,
    /// Set by fb_sync host function to trigger immediate render.
    pub force_render: bool,
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
    /// struct MyHostState { counter: u32 }
    ///
    /// // Store it during setup
    /// caller.data_mut().insert_custom(MyHostState { counter: 0 });
    ///
    /// // Read it in a host function
    /// let state = caller.data().get_custom::<MyHostState>().unwrap();
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
        renderer: FramebufferRenderer,
        filesystem: FileSystem,
        redstone: RedstoneState,
        interrupt_queue: InterruptQueue,
        input_rx: Receiver<String>,
        shutdown: Arc<AtomicBool>,
        network: Option<NetworkState>,
        engine: &Engine,
    ) -> Result<Self> {
        let module = Module::from_file(engine, wasm_path)?;

        let mut state = HostState {
            renderer,
            filesystem,
            redstone,
            interrupt_queue,
            input_rx: Arc::new(ChannelReceiver(std::sync::Mutex::new(input_rx))),
            shutdown,
            last_interrupt_payload_len: 0,
            next_object_handle: 1,
            force_render: false,
            custom: HashMap::new(),
        };

        // Insert FD table for kernel file descriptor operations
        state.insert_custom(crate::fd::FdTable::new());

        // Insert TTY registry for virtual terminal management
        state.insert_custom(Arc::new(std::sync::Mutex::new(crate::tty::TtyRegistry::new())));

        // Insert ProcessManager wrapped in Arc<Mutex<>> so it can be shared with spawned processes
        {
            use crate::process::ProcessManager;
            use std::sync::Mutex;
            let mut pm = ProcessManager::new(engine.clone());
            pm.register_kernel();
            state.insert_custom(Arc::new(Mutex::new(pm)));
        }

        // Insert network state if provided
        if let Some(net) = network {
            state.insert_custom(net);
        }

        let mut store = Store::new(engine, state);
        let mut linker = Linker::new(engine);

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
        self.render_framebuffer()?;
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

        self.render_framebuffer()?;
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

        self.render_framebuffer()?;
        Ok(())
    }

    /// Read the framebuffer from WASM memory and render it to the real terminal.
    pub fn render_framebuffer(&mut self) -> Result<()> {
        use crate::terminal_io::{FB_BASE, CELL_SIZE, GFX_BASE, GFX_HEADER_SIZE, GFX_PIXEL_OFF};

        let memory = match self.instance.get_memory(&mut self.store, "memory") {
            Some(m) => m,
            None => return Ok(()),
        };

        // Copy the framebuffer region from WASM memory to avoid borrow conflict.
        // We read the header to determine the size, then copy header + cells.
        let data = memory.data(&self.store);
        if data.len() < FB_BASE + 64 {
            return Ok(());
        }
        // Read width and height from header to determine copy size
        let width = u16::from_le_bytes([data[FB_BASE + 2], data[FB_BASE + 3]]) as usize;
        let height = u16::from_le_bytes([data[FB_BASE + 4], data[FB_BASE + 5]]) as usize;
        let total_size = 64 + width * height * CELL_SIZE;
        let end = FB_BASE + total_size;
        if end > data.len() {
            return Ok(());
        }
        let fb_copy = data[FB_BASE..end].to_vec();

        // Also read the graphics framebuffer if present
        let gfx_copy = if data.len() >= GFX_BASE + GFX_HEADER_SIZE {
            let gfx_magic = u16::from_le_bytes([data[GFX_BASE], data[GFX_BASE + 1]]);
            if gfx_magic == 0xFB02 {
                let gfx_w = u16::from_le_bytes([data[GFX_BASE + 4], data[GFX_BASE + 5]]) as usize;
                let gfx_h = u16::from_le_bytes([data[GFX_BASE + 6], data[GFX_BASE + 7]]) as usize;
                let gfx_total = GFX_PIXEL_OFF + gfx_w * gfx_h;
                let gfx_end = GFX_BASE + gfx_total;
                if gfx_end <= data.len() {
                    Some(data[GFX_BASE..gfx_end].to_vec())
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };
        // data borrow ends here (all needed slices are copied)

        // Update graphics state
        let renderer = &mut self.store.data_mut().renderer;
        if let Some(ref gfx) = gfx_copy {
            renderer.update_gfx_state(gfx);
        } else {
            renderer.display_mode = 0;
        }

        // Render the text framebuffer (handles display mode internally)
        self.store.data_mut().renderer.render_from_fb_slice(&fb_copy)?;
        self.store.data_mut().force_render = false;
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

            // Always render — the framebuffer may have been updated by
            // child processes, interrupts, or async operations without
            // any user input.
            if let Err(e) = self.render_framebuffer() {
                eprintln!("[Simulator] Error rendering: {}", e);
            }
        }
    }
}
