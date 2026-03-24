#![allow(dead_code)]

mod filesystem;
mod host;
mod interrupts;
mod redstone;
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
use crate::interrupts::InterruptQueue;
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

    let shutdown = Arc::new(AtomicBool::new(false));
    let interrupt_queue = InterruptQueue::new();
    let redstone = RedstoneState::new();
    let headless = cli.headless;

    let (input_tx, input_rx) = mpsc::channel::<String>();

    if !headless {
        // Enter raw terminal mode
        terminal_io::enter_raw_mode()?;

        // Set up panic hook to restore terminal
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
        ) {
            Ok(mut host) => {
                // Call main() to show boot banner
                if let Err(e) = host.call_main() {
                    eprintln!("[Simulator] Error calling main(): {}", e);
                    shutdown_worker.store(true, Ordering::Relaxed);
                    return;
                }

                // Enter worker loop
                host.worker_loop();
            }
            Err(e) => {
                shutdown_worker.store(true, Ordering::Relaxed);
                // Restore terminal before printing error
                let _ = terminal_io::exit_raw_mode();
                eprintln!("Failed to load WASM module: {}", e);
            }
        }
    });

    if headless {
        // Headless mode: read lines from stdin (for testing/piping)
        let shutdown_main = shutdown.clone();
        use std::io::BufRead;

        // Wait a moment for the worker to initialize and call main()
        std::thread::sleep(std::time::Duration::from_millis(500));

        let stdin = std::io::stdin();
        for line in stdin.lock().lines() {
            if shutdown_main.load(Ordering::Relaxed) {
                break;
            }
            match line {
                Ok(l) => {
                    // Send each character individually, then newline
                    for ch in l.chars() {
                        let _ = input_tx.send(ch.to_string());
                    }
                    let _ = input_tx.send("\n".to_string());
                    // Give the worker time to process
                    std::thread::sleep(std::time::Duration::from_millis(200));
                }
                Err(_) => break,
            }
        }
        // Wait for worker to finish processing
        std::thread::sleep(std::time::Duration::from_millis(500));
    } else {
        // Interactive mode: read keystrokes in raw terminal mode
        let interrupt_queue_main = interrupt_queue.clone();
        let redstone_main = redstone.clone();
        let shutdown_main = shutdown.clone();

        let mut redstone_mode = false;
        let mut redstone_input_buf = String::new();

        while !shutdown_main.load(Ordering::Relaxed) {
            if event::poll(std::time::Duration::from_millis(50))? {
                if let Event::Key(key_event) = event::read()? {
                    if redstone_mode {
                        handle_redstone_input(
                            key_event,
                            &mut redstone_mode,
                            &mut redstone_input_buf,
                            &redstone_main,
                            &interrupt_queue_main,
                            &input_tx,
                        );
                        continue;
                    }

                    match key_event {
                        KeyEvent { code: KeyCode::Char('q'), modifiers, .. }
                            if modifiers.contains(KeyModifiers::CONTROL) =>
                        {
                            shutdown_main.store(true, Ordering::Relaxed);
                            break;
                        }
                        KeyEvent { code: KeyCode::Char('t'), modifiers, .. }
                            if modifiers.contains(KeyModifiers::CONTROL) =>
                        {
                            interrupt_queue_main.push(
                                interrupts::IRQ_TERMINATE,
                                "{}".to_string(),
                            );
                            let _ = input_tx.send("\x14".to_string());
                        }
                        KeyEvent { code: KeyCode::Char('c'), modifiers, .. }
                            if modifiers.contains(KeyModifiers::CONTROL) =>
                        {
                            // Send Ctrl+C (0x03) to WASM — used as VIM ESC alternative
                            let _ = input_tx.send("\x03".to_string());
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
                        KeyEvent { code: KeyCode::Esc, .. } => {
                            let _ = input_tx.send("\x1b".to_string());
                        }
                        KeyEvent { code: KeyCode::Up, .. } => {
                            let _ = input_tx.send("\x1b[A".to_string());
                        }
                        KeyEvent { code: KeyCode::Down, .. } => {
                            let _ = input_tx.send("\x1b[B".to_string());
                        }
                        KeyEvent { code: KeyCode::Left, .. } => {
                            let _ = input_tx.send("\x1b[D".to_string());
                        }
                        KeyEvent { code: KeyCode::Right, .. } => {
                            let _ = input_tx.send("\x1b[C".to_string());
                        }
                        KeyEvent { code: KeyCode::Delete, .. } => {
                            let _ = input_tx.send("\x1b[3~".to_string());
                        }
                        KeyEvent { code: KeyCode::Home, .. } => {
                            let _ = input_tx.send("\x1b[H".to_string());
                        }
                        KeyEvent { code: KeyCode::End, .. } => {
                            let _ = input_tx.send("\x1b[F".to_string());
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    // Cleanup
    if !headless {
        let _ = terminal_io::exit_raw_mode();
    }

    // Wait for worker thread
    drop(input_tx);
    let _ = worker_handle.join();

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

                    // Queue IRQ_REDSTONE
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
