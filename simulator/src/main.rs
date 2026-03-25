#![allow(dead_code)]

mod filesystem;
mod host;
mod hub;
mod interrupts;
mod network;
mod redstone;
mod tap;
mod terminal_io;
mod wasm_bindgen_stubs;
mod wasm_host;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;

use clap::Parser;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};

use crate::filesystem::FileSystem;
use crate::hub::EthernetHub;
use crate::interrupts::InterruptQueue;
use crate::network::{NetworkState, mac_for_instance};
use crate::redstone::RedstoneState;
use crate::terminal_io::TerminalBuffer;
#[allow(unused_imports)]
use crate::wasm_host::WasmHost;

#[derive(Parser)]
#[command(name = "terminal-simulator")]
#[command(about = "CLI simulator for the Terminal OS WASM module")]
struct Cli {
    /// Path to the terminal_os.wasm file
    #[arg(long, default_value = "../wasm-bin/terminal_os.wasm")]
    wasm: PathBuf,

    /// Directory for file storage
    #[arg(long, default_value = "./simulator-data")]
    storage: PathBuf,

    /// Terminal width
    #[arg(long, default_value_t = 80)]
    width: usize,

    /// Terminal height
    #[arg(long, default_value_t = 24)]
    height: usize,

    /// Run in headless mode (no raw terminal, for testing)
    #[arg(long)]
    headless: bool,

    /// Number of WASM instances to run (each gets its own computer)
    #[arg(long, default_value_t = 1)]
    instances: u8,

    /// Auto-configure networking (instance i gets IP 10.0.0.(i+1)/24)
    #[arg(long)]
    auto_net: bool,

    /// Script files to run on specific instances (e.g., --script 0:server.sh --script 1:client.sh)
    /// Format: "instance_index:filepath" or just "filepath" (runs on instance 0)
    #[arg(long)]
    script: Vec<String>,

    /// Enable TAP bridge for real internet access (e.g., --tap tap0)
    /// Requires root or CAP_NET_ADMIN. Linux only.
    #[arg(long)]
    tap: Option<String>,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    if !cli.wasm.exists() {
        eprintln!("Error: WASM file not found: {}", cli.wasm.display());
        eprintln!("Make sure terminal_os.wasm is built:");
        eprintln!("  cd operating-system/rust");
        eprintln!("  cargo build --release --target wasm32-unknown-unknown");
        eprintln!("  cp target/wasm32-unknown-unknown/release/terminal_os.wasm ../../wasm-bin/");
        std::process::exit(1);
    }

    let num_instances = cli.instances.max(1);
    let shutdown = Arc::new(AtomicBool::new(false));
    let headless = cli.headless || num_instances > 1; // multi-instance forces headless for non-primary

    // Create the shared ethernet hub
    let hub = EthernetHub::new();

    // Set up TAP bridge if requested
    let mut tap_handles: Vec<std::thread::JoinHandle<()>> = Vec::new();
    if let Some(ref tap_name) = cli.tap {
        match tap::TapDevice::open(tap_name) {
            Ok(tap_dev) => {
                eprintln!("[TAP] Opened device: {}", tap_dev.name());
                hub.set_tap_attached(true);

                let hub_reader = hub.clone();
                let hub_writer = hub.clone();
                let shutdown_reader = shutdown.clone();
                let shutdown_writer = shutdown.clone();

                // Clone the TAP device fd for the reader thread
                let mut tap_read = tap_dev.try_clone()
                    .expect("[TAP] Failed to dup() TAP fd");
                let mut tap_write = tap_dev;

                // TAP reader thread: reads frames from TAP, injects into hub
                let reader_handle = std::thread::spawn(move || {
                    let mut buf = [0u8; 1518]; // 802.1Q max
                    while !shutdown_reader.load(Ordering::Relaxed) {
                        match tap_read.recv_frame(&mut buf) {
                            Ok(Some(len)) => {
                                hub_reader.inject_from_tap(&buf[..len]);
                            }
                            Ok(None) => {
                                std::thread::sleep(std::time::Duration::from_millis(1));
                            }
                            Err(e) => {
                                if !shutdown_reader.load(Ordering::Relaxed) {
                                    eprintln!("[TAP] Read error: {}", e);
                                }
                                break;
                            }
                        }
                    }
                });
                tap_handles.push(reader_handle);

                // TAP writer thread: drains hub's TAP tx queue, writes to TAP
                let writer_handle = std::thread::spawn(move || {
                    while !shutdown_writer.load(Ordering::Relaxed) {
                        match hub_writer.pop_tap_frame(std::time::Duration::from_millis(50)) {
                            Some(frame) => {
                                if let Err(e) = tap_write.send_frame(&frame) {
                                    if !shutdown_writer.load(Ordering::Relaxed) {
                                        eprintln!("[TAP] Write error: {}", e);
                                    }
                                }
                            }
                            None => {} // timeout, loop
                        }
                    }
                });
                tap_handles.push(writer_handle);
            }
            Err(e) => {
                eprintln!("[TAP] Failed to open {}: {}", tap_name, e);
                eprintln!("[TAP] Internet access disabled. Continuing without TAP.");
            }
        }
    }

    // For single instance, behave exactly like before (backwards compatible)
    let result = if num_instances == 1 {
        run_single_instance(&cli, shutdown.clone(), hub, headless)
    } else {
        // Multi-instance mode: all instances are headless
        run_multi_instance(&cli, num_instances, shutdown.clone(), hub)
    };

    // Clean up TAP threads
    shutdown.store(true, Ordering::Relaxed);
    for handle in tap_handles {
        let _ = handle.join();
    }

    result
}

/// Original single-instance mode (backwards compatible).
fn run_single_instance(
    cli: &Cli,
    shutdown: Arc<AtomicBool>,
    hub: Arc<EthernetHub>,
    headless: bool,
) -> anyhow::Result<()> {
    let interrupt_queue = InterruptQueue::new();
    let redstone = RedstoneState::new();

    // Register NIC on the hub
    let mac = mac_for_instance(0);
    hub.register_nic(mac, interrupt_queue.clone());
    let net_state = NetworkState::new(mac, hub);

    let (input_tx, input_rx) = mpsc::channel::<String>();

    if !headless {
        terminal_io::enter_raw_mode()?;

        let shutdown_panic = shutdown.clone();
        let original_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            shutdown_panic.store(true, Ordering::Relaxed);
            let _ = terminal_io::exit_raw_mode();
            original_hook(info);
        }));
    }

    // Spawn WASM worker thread
    let shutdown_worker = shutdown.clone();
    let wasm_path = cli.wasm.clone();
    let interrupt_queue_worker = interrupt_queue.clone();
    let redstone_worker = redstone.clone();
    let width = cli.width;
    let height = cli.height;
    let storage = cli.storage.clone();
    let auto_net = cli.auto_net;

    let worker_handle = std::thread::spawn(move || {
        let mut terminal = TerminalBuffer::new(width, height);
        terminal.headless = headless;
        let filesystem = FileSystem::new(storage);

        match WasmHost::new(
            &wasm_path,
            terminal,
            filesystem,
            redstone_worker,
            interrupt_queue_worker,
            input_rx,
            shutdown_worker.clone(),
            Some(net_state),
        ) {
            Ok(mut host) => {
                if let Err(e) = host.call_main() {
                    eprintln!("[Simulator] Error calling main(): {}", e);
                    shutdown_worker.store(true, Ordering::Relaxed);
                    return;
                }

                // If auto-net, send ifconfig command
                if auto_net {
                    let cmd = "ifconfig set 10.0.0.1 255.255.255.0 10.0.0.254 8.8.8.8\n";
                    for ch in cmd.chars() {
                        if let Err(e) = host.send_input(&ch.to_string()) {
                            eprintln!("[Simulator] Error sending auto-net config: {}", e);
                        }
                    }
                }

                host.worker_loop();
            }
            Err(e) => {
                shutdown_worker.store(true, Ordering::Relaxed);
                let _ = terminal_io::exit_raw_mode();
                eprintln!("Failed to load WASM module: {}", e);
            }
        }
    });

    if headless {
        run_headless_input(&shutdown, &input_tx);
    } else {
        run_interactive_input(&shutdown, &interrupt_queue, &redstone, &input_tx)?;
    }

    // Cleanup
    if !headless {
        let _ = terminal_io::exit_raw_mode();
    }

    drop(input_tx);
    let _ = worker_handle.join();

    Ok(())
}

/// Multi-instance mode: all instances run headless.
fn run_multi_instance(
    cli: &Cli,
    num_instances: u8,
    shutdown: Arc<AtomicBool>,
    hub: Arc<EthernetHub>,
) -> anyhow::Result<()> {
    let mut worker_handles = Vec::new();
    let mut input_txs = Vec::new();

    for i in 0..num_instances {
        let mac = mac_for_instance(i);
        let interrupt_queue = InterruptQueue::new();
        hub.register_nic(mac, interrupt_queue.clone());
        let net_state = NetworkState::new(mac, hub.clone());

        let (input_tx, input_rx) = mpsc::channel::<String>();
        input_txs.push(input_tx);

        let shutdown_worker = shutdown.clone();
        let wasm_path = cli.wasm.clone();
        let width = cli.width;
        let height = cli.height;
        let storage = cli.storage.join(format!("{}", i));
        let redstone = RedstoneState::new();
        let auto_net = cli.auto_net;

        let handle = std::thread::spawn(move || {
            let mut terminal = TerminalBuffer::new(width, height);
            terminal.headless = true;
            let filesystem = FileSystem::new(storage);

            match WasmHost::new(
                &wasm_path,
                terminal,
                filesystem,
                redstone,
                interrupt_queue,
                input_rx,
                shutdown_worker.clone(),
                Some(net_state),
            ) {
                Ok(mut host) => {
                    if let Err(e) = host.call_main() {
                        eprintln!("[Instance {}] Error calling main(): {}", i, e);
                        shutdown_worker.store(true, Ordering::Relaxed);
                        return;
                    }

                    // Auto-configure networking
                    if auto_net {
                        let ip_last = i + 1;
                        let cmd = format!(
                            "ifconfig set 10.0.0.{} 255.255.255.0 10.0.0.254 8.8.8.8\n",
                            ip_last
                        );
                        for ch in cmd.chars() {
                            if let Err(e) = host.send_input(&ch.to_string()) {
                                eprintln!("[Instance {}] Error sending auto-net config: {}", i, e);
                            }
                        }
                    }

                    host.worker_loop();
                }
                Err(e) => {
                    shutdown_worker.store(true, Ordering::Relaxed);
                    eprintln!("[Instance {}] Failed to load WASM module: {}", i, e);
                }
            }
        });

        worker_handles.push(handle);
    }

    // Parse per-instance scripts
    let mut instance_scripts: Vec<Option<String>> = vec![None; num_instances as usize];
    for script_arg in &cli.script {
        if let Some(colon_pos) = script_arg.find(':') {
            if let Ok(idx) = script_arg[..colon_pos].parse::<usize>() {
                let path = &script_arg[colon_pos + 1..];
                if let Ok(content) = std::fs::read_to_string(path) {
                    instance_scripts[idx] = Some(content);
                } else {
                    eprintln!("[Simulator] Failed to read script: {}", path);
                }
            }
        } else {
            // No index prefix — run on instance 0
            if let Ok(content) = std::fs::read_to_string(script_arg) {
                instance_scripts[0] = Some(content);
            }
        }
    }

    // Send scripts to their respective instances
    for (idx, script) in instance_scripts.iter().enumerate() {
        if let Some(content) = script {
            let tx = &input_txs[idx];
            // Wait for instance to boot
            std::thread::sleep(std::time::Duration::from_millis(800));
            for line in content.lines() {
                for ch in line.chars() {
                    let _ = tx.send(ch.to_string());
                }
                let _ = tx.send("\n".to_string());
                std::thread::sleep(std::time::Duration::from_millis(200));
            }
        }
    }

    // In multi-instance headless mode, read stdin and send to instance 0
    eprintln!(
        "[Simulator] Running {} instances in headless mode. Stdin routes to instance 0.",
        num_instances
    );

    if !input_txs.is_empty() {
        run_headless_input(&shutdown, &input_txs[0]);
    }

    // Wait a moment for final processing, then shutdown
    std::thread::sleep(std::time::Duration::from_millis(500));
    shutdown.store(true, Ordering::Relaxed);

    for tx in input_txs {
        drop(tx);
    }
    for handle in worker_handles {
        let _ = handle.join();
    }

    Ok(())
}

/// Read lines from stdin and send to the given input channel (headless mode).
fn run_headless_input(shutdown: &Arc<AtomicBool>, input_tx: &mpsc::Sender<String>) {
    use std::io::BufRead;

    // Wait for the worker to initialize
    std::thread::sleep(std::time::Duration::from_millis(500));

    let stdin = std::io::stdin();
    for line in stdin.lock().lines() {
        if shutdown.load(Ordering::Relaxed) {
            break;
        }
        match line {
            Ok(l) => {
                for ch in l.chars() {
                    let _ = input_tx.send(ch.to_string());
                }
                let _ = input_tx.send("\n".to_string());
                std::thread::sleep(std::time::Duration::from_millis(200));
            }
            Err(_) => break,
        }
    }
    std::thread::sleep(std::time::Duration::from_millis(500));
}

/// Interactive raw terminal input loop.
fn run_interactive_input(
    shutdown: &Arc<AtomicBool>,
    interrupt_queue: &InterruptQueue,
    redstone: &RedstoneState,
    input_tx: &mpsc::Sender<String>,
) -> anyhow::Result<()> {
    let mut redstone_mode = false;
    let mut redstone_input_buf = String::new();

    while !shutdown.load(Ordering::Relaxed) {
        if event::poll(std::time::Duration::from_millis(50))? {
            if let Event::Key(key_event) = event::read()? {
                if redstone_mode {
                    handle_redstone_input(
                        key_event,
                        &mut redstone_mode,
                        &mut redstone_input_buf,
                        redstone,
                        interrupt_queue,
                        input_tx,
                    );
                    continue;
                }

                match key_event {
                    KeyEvent { code: KeyCode::Char('q'), modifiers, .. }
                        if modifiers.contains(KeyModifiers::CONTROL) =>
                    {
                        shutdown.store(true, Ordering::Relaxed);
                        break;
                    }
                    KeyEvent { code: KeyCode::Char('t'), modifiers, .. }
                        if modifiers.contains(KeyModifiers::CONTROL) =>
                    {
                        interrupt_queue.push(
                            interrupts::IRQ_TERMINATE,
                            "{}".to_string(),
                        );
                        let _ = input_tx.send("\x14".to_string());
                    }
                    KeyEvent { code: KeyCode::Char('c'), modifiers, .. }
                        if modifiers.contains(KeyModifiers::CONTROL) =>
                    {
                        interrupt_queue.push(
                            interrupts::IRQ_TERMINATE,
                            "{}".to_string(),
                        );
                        let _ = input_tx.send("\x14".to_string());
                    }
                    KeyEvent { code: KeyCode::Char('r'), modifiers, .. }
                        if modifiers.contains(KeyModifiers::CONTROL) =>
                    {
                        redstone_mode = true;
                        redstone_input_buf.clear();
                        eprintln!("\n[Redstone] Enter 'side power' (e.g., '3 15' for BACK=15), or 'q' to cancel:");
                    }
                    KeyEvent { code: KeyCode::Char('d'), modifiers, .. }
                        if modifiers.contains(KeyModifiers::CONTROL) =>
                    {
                        let _ = input_tx.send("\x04".to_string());
                    }
                    KeyEvent { code: KeyCode::Char(c), modifiers, .. } => {
                        if modifiers.contains(KeyModifiers::CONTROL) {
                            let ctrl_char = (c as u8 - b'a' + 1) as char;
                            let _ = input_tx.send(ctrl_char.to_string());
                        } else {
                            let _ = input_tx.send(c.to_string());
                        }
                    }
                    KeyEvent { code: KeyCode::Enter, .. } => {
                        let _ = input_tx.send("\n".to_string());
                    }
                    KeyEvent { code: KeyCode::Backspace, .. } => {
                        let _ = input_tx.send("\x7f".to_string());
                    }
                    KeyEvent { code: KeyCode::Tab, .. } => {
                        let _ = input_tx.send("\t".to_string());
                    }
                    _ => {}
                }
            }
        }
    }

    Ok(())
}

fn handle_redstone_input(
    key_event: KeyEvent,
    redstone_mode: &mut bool,
    buf: &mut String,
    redstone: &RedstoneState,
    interrupt_queue: &InterruptQueue,
    _input_tx: &mpsc::Sender<String>,
) {
    match key_event.code {
        KeyCode::Char('q') if buf.is_empty() => {
            *redstone_mode = false;
            eprintln!("[Redstone] Cancelled");
        }
        KeyCode::Char(c) => {
            buf.push(c);
        }
        KeyCode::Backspace => {
            buf.pop();
        }
        KeyCode::Enter => {
            let parts: Vec<&str> = buf.trim().split_whitespace().collect();
            if parts.len() == 2 {
                if let (Ok(side), Ok(power)) = (parts[0].parse::<i32>(), parts[1].parse::<i32>()) {
                    let old_inputs = redstone.set_input(side, power);
                    let new_inputs = redstone.get_all_inputs();

                    let payload = format!(
                        "{{\"sides\":[{},{},{},{},{},{}],\"old_sides\":[{},{},{},{},{},{}]}}",
                        new_inputs[0], new_inputs[1], new_inputs[2],
                        new_inputs[3], new_inputs[4], new_inputs[5],
                        old_inputs[0], old_inputs[1], old_inputs[2],
                        old_inputs[3], old_inputs[4], old_inputs[5],
                    );
                    interrupt_queue.push(interrupts::IRQ_REDSTONE, payload);
                    eprintln!("[Redstone] Set side {} = {}", side, power);
                } else {
                    eprintln!("[Redstone] Invalid input. Use: side power (e.g., 3 15)");
                }
            } else {
                eprintln!("[Redstone] Invalid input. Use: side power (e.g., 3 15)");
            }
            *redstone_mode = false;
            buf.clear();
        }
        KeyCode::Esc => {
            *redstone_mode = false;
            eprintln!("[Redstone] Cancelled");
        }
        _ => {}
    }
}
