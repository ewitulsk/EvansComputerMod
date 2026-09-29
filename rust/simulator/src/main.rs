//! terminal-simulator: runs the real `terminal_os.wasm` kernel and real WASI
//! programs without Minecraft, on a simulated cable topology.
//!
//! Modes:
//! - `--scenario file.toml`: headless scenario (topology + steps); exit 0 on
//!   success, 1 on failure (with a screen dump), 2 on bad input, 3 when the
//!   kernel/programs are not built.
//! - interactive (default): one terminal view per node, switchable.
//! - `--check-programs`: link-check every program against the WASI host.

mod child;
mod fs;
mod interactive;
mod ipc;
mod kernel;
mod net;
mod pcap;
mod proc;
mod runtime;
mod scenario;
mod sched;
mod screen;
mod sim;
mod topology;
mod util;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use clap::Parser;

use crate::sched::ClockMode;
use crate::sim::{Sim, SimConfig};
use crate::topology::TopoFile;
use crate::util::fmt_ms;

#[derive(Parser)]
#[command(name = "terminal-simulator", about = "Host for terminal_os.wasm: topology, virtual clock, scenarios")]
struct Cli {
    /// Run a headless scenario (topology + [scenario] steps).
    #[arg(long)]
    scenario: Option<PathBuf>,
    /// Topology file for interactive mode.
    #[arg(long)]
    topology: Option<PathBuf>,
    /// Interactive mode without a topology: N unconnected computers pc1..pcN.
    #[arg(long, default_value_t = 1)]
    nodes: usize,
    /// Kernel wasm [default: rust/target/wasm32-unknown-unknown/release/terminal_os.wasm].
    #[arg(long)]
    kernel: Option<PathBuf>,
    /// Directory of WASI programs, mounted read-only at server-bin/
    /// [default: rust/target/wasm32-wasip1/release].
    #[arg(long)]
    programs: Option<PathBuf>,
    /// Storage root (one subdirectory per node). Scenarios default to a
    /// fresh temporary directory; interactive mode to ./simulator-data.
    #[arg(long)]
    storage: Option<PathBuf>,
    /// Where pcap files go [default: ./sim-out].
    #[arg(long)]
    out_dir: Option<PathBuf>,
    /// real | virtual [default: virtual for scenarios, real interactively].
    #[arg(long)]
    clock: Option<String>,
    /// PRNG seed for fault injection and guest entropy.
    #[arg(long)]
    seed: Option<u64>,
    /// Keep the scenario's temporary storage directory.
    #[arg(long)]
    keep_storage: bool,
    /// Busy rounds per virtual millisecond before the clock is forced on.
    #[arg(long, default_value_t = 16)]
    max_rounds_per_ms: u32,
    /// Kernel call watchdog (trap + kernel_recover), e.g. "10s".
    #[arg(long, default_value = "10s")]
    watchdog: String,
    /// Scenario: don't echo steps.
    #[arg(long)]
    quiet: bool,
    /// Print host log lines as they happen (same as ECM_SIM_VERBOSE=1).
    #[arg(long)]
    verbose: bool,
    /// Check that every program in --programs links against the WASI host.
    #[arg(long)]
    check_programs: bool,
}

fn manifest_default(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(rel)
}

pub fn default_kernel() -> PathBuf {
    manifest_default("../target/wasm32-unknown-unknown/release/terminal_os.wasm")
}

pub fn default_programs() -> PathBuf {
    manifest_default("../target/wasm32-wasip1/release")
}

pub const BUILD_KERNEL: &str = "cd rust && cargo build --release --target wasm32-unknown-unknown -p terminal-os";
pub const BUILD_PROGRAMS: &str = "cd rust && cargo build --release --target wasm32-wasip1 -p echo -p sleep -p ifconfig -p ping -p curl -p httpd -p help -p ls -p cat";

fn exit_with(code: i32, msg: &str) -> ! {
    eprintln!("{}", msg);
    std::process::exit(code)
}

fn resolve_rel(base: &Path, p: &str) -> PathBuf {
    let pp = Path::new(p);
    if pp.is_absolute() {
        pp.to_path_buf()
    } else {
        base.join(pp)
    }
}

fn check_built(kernel: &Path, programs: &Path) {
    if !kernel.is_file() {
        exit_with(3, &format!("kernel not found: {}\nbuild it with:\n  {}", kernel.display(), BUILD_KERNEL));
    }
    if !programs.join("echo.wasm").is_file() {
        exit_with(
            3,
            &format!("WASI programs not found in {}\nbuild them with:\n  {}", programs.display(), BUILD_PROGRAMS),
        );
    }
}

fn main() {
    let cli = Cli::parse();
    if cli.verbose {
        std::env::set_var("ECM_SIM_VERBOSE", "1");
    }
    let watchdog = util::parse_duration_ms(&cli.watchdog).unwrap_or_else(|| exit_with(2, "bad --watchdog"));

    if cli.check_programs {
        let dir = cli.programs.clone().unwrap_or_else(default_programs);
        std::process::exit(check_programs(&dir));
    }

    let (topo, is_scenario) = match (&cli.scenario, &cli.topology) {
        (Some(p), _) => (TopoFile::load(p).unwrap_or_else(|e| exit_with(2, &e)), true),
        (None, Some(p)) => (TopoFile::load(p).unwrap_or_else(|e| exit_with(2, &e)), false),
        (None, None) => (TopoFile::unconnected(cli.nodes), false),
    };
    let base = topo.dir.clone();
    let kernel = cli
        .kernel
        .clone()
        .or_else(|| topo.sim.kernel.as_ref().map(|k| resolve_rel(&base, k)))
        .unwrap_or_else(default_kernel);
    let programs = cli
        .programs
        .clone()
        .or_else(|| topo.sim.programs.as_ref().map(|k| resolve_rel(&base, k)))
        .unwrap_or_else(default_programs);
    check_built(&kernel, &programs);

    let clock = match cli.clock.as_deref().or(topo.sim.clock.as_deref()) {
        Some("real") => ClockMode::Real,
        Some("virtual") => ClockMode::Virtual,
        None if is_scenario => ClockMode::Virtual,
        None => ClockMode::Real,
        Some(o) => exit_with(2, &format!("--clock must be real or virtual, not '{}'", o)),
    };
    let seed = cli.seed.or(topo.sim.seed).unwrap_or(1);
    let temp_storage = cli.storage.is_none() && is_scenario;
    let storage = cli.storage.clone().unwrap_or_else(|| {
        if is_scenario {
            std::env::temp_dir().join(format!(
                "ecm-sim-{}-{}",
                std::process::id(),
                Instant::now().elapsed().as_nanos() ^ (seed as u128)
            ))
        } else {
            PathBuf::from("simulator-data")
        }
    });
    let out_dir = cli.out_dir.clone().unwrap_or_else(|| PathBuf::from("sim-out"));
    let watchdog_ms = topo.sim.watchdog.as_ref().and_then(|d| d.ms().ok()).unwrap_or(watchdog);
    let cfg = SimConfig {
        clock,
        seed,
        kernel,
        programs: Some(programs),
        storage: storage.clone(),
        out_dir,
        watchdog: Duration::from_millis(watchdog_ms as u64),
        stall: Duration::from_secs(30),
        max_rounds_per_ms: cli.max_rounds_per_ms.max(1),
    };

    let code = if is_scenario {
        run_scenario(cfg, &topo, cli.scenario.as_deref().unwrap(), cli.quiet)
    } else {
        match interactive::run(cfg, &topo) {
            Ok(()) => 0,
            Err(e) => {
                eprintln!("error: {:#}", e);
                1
            }
        }
    };
    if temp_storage && !cli.keep_storage {
        let _ = std::fs::remove_dir_all(&storage);
    } else if temp_storage {
        eprintln!("storage kept at {}", storage.display());
    }
    std::process::exit(code);
}

fn run_scenario(cfg: SimConfig, topo: &TopoFile, path: &Path, quiet: bool) -> i32 {
    let Some(sc) = topo.scenario.clone() else {
        eprintln!("{}: no [scenario] section", path.display());
        return 2;
    };
    let steps = match scenario::parse_steps(&sc.steps) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{}: {}", path.display(), e);
            return 2;
        }
    };
    let name = sc.name.clone().unwrap_or_else(|| path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default());
    let default_timeout = topo.sim.timeout.as_ref().and_then(|d| d.ms().ok()).unwrap_or(10_000);
    let wall = sc.wall_timeout.as_ref().and_then(|d| d.ms().ok()).unwrap_or(300_000);
    let t0 = Instant::now();
    if !quiet {
        println!("== scenario {} ({} nodes, clock {:?}, seed {})", name, topo.node.len(), cfg.clock, cfg.seed);
        if let Some(d) = &sc.description {
            println!("   {}", d.trim());
        }
    }
    let mut sim = match Sim::new(cfg, topo) {
        Ok(s) => s,
        Err(e) => {
            println!("FAILED {}: setup: {:#}", name, e);
            return 1;
        }
    };
    sim.wall_deadline = Some(Instant::now() + Duration::from_millis(wall as u64));
    // Boot: every node shows its first prompt, then its boot commands run.
    let boot = (|| -> Result<(), String> {
        for i in 0..sim.nodes.len() {
            let d = sim.now() + 30_000;
            if !sim.run_until(d, |s| s.nodes[i].kernel.faulted.is_some() || s.at_prompt(i))? {
                return Err(format!("{}: no prompt after boot", sim.nodes[i].name));
            }
            if let Some(f) = &sim.nodes[i].kernel.faulted {
                return Err(format!("{}: {}", sim.nodes[i].name, f));
            }
        }
        sim.run_boot_commands(30_000)
    })();
    let result = match boot {
        Err(e) => Err(scenario::Failure {
            line: scenario::Line { no: 0, text: "(boot)".into(), step: scenario::Step::Log { text: String::new() } },
            reason: e,
            node: None,
        }),
        Ok(()) => {
            let mut printer = |s: String| {
                if !quiet {
                    println!("{}", s);
                }
            };
            scenario::run_steps(&mut sim, &steps, default_timeout, &mut printer)
        }
    };
    let code = match result {
        Ok(()) => {
            println!(
                "PASS {} (virtual {}, wall {:.1}s)",
                name,
                fmt_ms(sim.sched.elapsed()),
                t0.elapsed().as_secs_f64()
            );
            for w in &sim.warnings {
                println!("warning: {}", w);
            }
            0
        }
        Err(f) => {
            println!("{}", scenario::failure_report(&sim, &f));
            println!("FAILED {} (virtual {}, wall {:.1}s)", name, fmt_ms(sim.sched.elapsed()), t0.elapsed().as_secs_f64());
            1
        }
    };
    sim.shutdown();
    code
}

fn check_programs(dir: &Path) -> i32 {
    let rt = match runtime::Runtime::new() {
        Ok(r) => r,
        Err(e) => exit_with(1, &format!("{:#}", e)),
    };
    let linker = match child::build_linker(&rt) {
        Ok(l) => l,
        Err(e) => exit_with(1, &format!("{:#}", e)),
    };
    let provided = child::provided_sigs_static(&rt, &linker);
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| exit_with(3, &format!("{}: {}", dir.display(), e)))
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "wasm"))
        .collect();
    entries.sort();
    let mut bad = 0;
    for p in entries {
        let name = p.file_name().unwrap().to_string_lossy().into_owned();
        match rt.module(&p) {
            Err(e) => {
                println!("{:<20} COMPILE ERROR {:#}", name, e);
                bad += 1;
            }
            Ok(m) => match runtime::check_imports(&m, &provided) {
                Ok(()) => println!("{:<20} ok", name),
                Err(e) => {
                    println!("{:<20} {}", name, e);
                    bad += 1;
                }
            },
        }
    }
    if bad > 0 {
        1
    } else {
        0
    }
}
