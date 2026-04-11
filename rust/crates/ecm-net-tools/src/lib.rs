//! Shared infrastructure for network CLI tools (ifconfig, ip, ping, nslookup,
//! resolvectl). Exposes a `NetTools` trait so the same program logic can run
//! either as a WASI binary (talking to the host via `ecm-host-abi` syscalls)
//! or baked into an in-kernel OS that calls the kernel netlink/NetStack
//! directly.

pub mod icmp;
pub mod nl;

// The WASI-side `NetTools` backend uses `std::io`/`std::thread` and the
// `ecm-host-abi` socket externs. It is only meaningful on WASI targets, so
// gate the module — in-kernel OSes (e.g. switch-os, wasm32-unknown-unknown)
// supply their own `NetTools` backend and must not try to link the WASI one.
#[cfg(target_os = "wasi")]
pub mod wasi;

/// Backend trait abstracting the host-interface operations networking CLI
/// tools need. One implementation (`WasiNetTools`) ships in this crate and is
/// used by the WASI program binaries. Kernel OSes (e.g. switch-os) provide
/// their own implementation that talks directly to the in-process NetStack.
pub trait NetTools {
    /// Write to standard output. Caller supplies any trailing newline.
    fn out(&mut self, s: &str);

    /// Write to standard error. Caller supplies any trailing newline.
    fn err(&mut self, s: &str);

    /// Milliseconds since an arbitrary epoch (monotonic-ish host clock).
    fn now_ms(&mut self) -> i64;

    /// Sleep for the given number of milliseconds.
    fn sleep_ms(&mut self, ms: u32);

    /// Send a netlink request and collect the full multi-part response
    /// (including the terminating NLMSG_DONE/ERROR message, if any).
    fn netlink_request(&mut self, req: &[u8]) -> Vec<u8>;

    /// Resolve a hostname to an IPv4 address. Returns None on failure.
    fn getaddrinfo(&mut self, host: &str) -> Option<[u8; 4]>;

    /// Send an ICMP echo request and wait up to `timeout_ms` for a reply.
    /// Returns the round-trip time in milliseconds on success.
    fn icmp_echo(&mut self, dst: [u8; 4], id: u16, seq: u16, timeout_ms: u32) -> Option<u32>;
}

/// Convenience macros-free formatting helpers for tool implementations.
/// Using these keeps call sites concise while still routing everything
/// through the `NetTools::out`/`err` channels.
pub fn out_line(nt: &mut dyn NetTools, s: &str) {
    nt.out(s);
    nt.out("\n");
}

pub fn err_line(nt: &mut dyn NetTools, s: &str) {
    nt.err(s);
    nt.err("\n");
}
