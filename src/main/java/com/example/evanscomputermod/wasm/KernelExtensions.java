package com.example.evanscomputermod.wasm;

import com.example.evanscomputermod.computer.ComputerInstance;

/**
 * KERN-042/043/044/045: New host functions for kernel-to-host communication.
 *
 * These functions extend ComputerInstance with:
 * - File descriptor operations (fd_open, fd_read, fd_write, fd_close, pipe_create)
 * - Process management (process_spawn, process_wait, process_kill, process_list)
 * - TTY management (tty_create, tty_attach_fd, tty_set_foreground)
 * - Socket operations (sock_tcp_connect, sock_tcp_listen, etc.)
 * - WASI compatibility functions (wasi_snapshot_preview1 namespace)
 *
 * Currently implemented in the Rust simulator (simulator/src/host/).
 * Java implementations will be added as the mod is brought to parity.
 *
 * See tickets.md KERN-042 through KERN-045 for full requirements.
 */
public class KernelExtensions {

    /**
     * Register all new kernel host functions on a ComputerInstance.
     * Call this from ComputerInstance.createHostFunctions() to add
     * the new FD, process, TTY, and socket host functions.
     *
     * @param instance The ComputerInstance to register functions on
     */
    public static void registerAll(ComputerInstance instance) {
        // TODO: Implement Java-side host functions matching simulator/src/host/
        //
        // Required modules:
        // - fd_ops: fd_open, fd_read, fd_write, fd_close, fd_dup, fd_dup2, pipe_create
        // - process: process_spawn, process_wait, process_kill, process_list, process_state
        // - tty: tty_create, tty_attach_fd, tty_set_foreground, tty_get_size, tty_write_input
        // - socket: sock_tcp_connect, sock_tcp_listen, sock_tcp_accept, sock_send, sock_recv
        //
        // For WASI binary support, also need:
        // - wasi_snapshot_preview1 namespace functions (fd_write, fd_read, args_get, etc.)
        // - ProcessManager (multiple WASM instances per computer)
        // - FdTable, PipeFd, VfsFileFd, TerminalFd implementations
        //
        // Reference implementation: simulator/src/host/ and simulator/src/process.rs
    }

    /**
     * List of new "env" namespace host function names.
     * These must be registered alongside existing host functions.
     */
    public static final String[] NEW_HOST_FUNCTIONS = {
        // FD operations
        "fd_open", "fd_read", "fd_write", "fd_close", "fd_dup", "fd_dup2", "pipe_create",
        // Process management
        "process_spawn", "process_wait", "process_wait_any", "process_kill",
        "process_list", "process_state", "process_exit",
        // TTY management
        "tty_create", "tty_attach_fd", "tty_set_foreground", "tty_get_size", "tty_write_input",
        // Socket operations
        "sock_tcp_connect", "sock_tcp_listen", "sock_tcp_accept",
        "sock_send", "sock_recv", "sock_shutdown",
    };
}
