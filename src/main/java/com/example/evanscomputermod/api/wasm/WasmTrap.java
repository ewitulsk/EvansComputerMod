package com.example.evanscomputermod.api.wasm;

/**
 * Unified runtime-agnostic trap. Wraps every kind of failure that can come
 * out of a WASM call so call sites don't have to know which provider raised
 * it. Each provider is responsible for classifying its native exception
 * types into a {@link Kind}.
 */
public final class WasmTrap extends RuntimeException {

    public enum Kind {
        /** The host requested cancellation via {@link WasmInstance#requestInterrupt()}. */
        INTERRUPTED,
        /** Guest invoked {@code proc_exit}. {@link #exitCode} is valid. */
        EXIT,
        /** Guest tried to grow memory past the limit, or the host ran out of heap. */
        OOM,
        /** Generic execution failure (divide-by-zero, OOB memory, unreachable, etc). */
        EXEC_ERROR,
        /** Module imports could not be resolved at instantiate time. */
        LINK_ERROR
    }

    private final Kind kind;
    private final int exitCode;

    public WasmTrap(Kind kind, String message) {
        super(message);
        this.kind = kind;
        this.exitCode = 0;
    }

    public WasmTrap(Kind kind, String message, Throwable cause) {
        super(message, cause);
        this.kind = kind;
        this.exitCode = 0;
    }

    /** Construct an EXIT trap. */
    public WasmTrap(int exitCode) {
        super("proc_exit(" + exitCode + ")");
        this.kind = Kind.EXIT;
        this.exitCode = exitCode;
    }

    public Kind kind() {
        return kind;
    }

    /** Exit code; only meaningful when {@link #kind()} is {@link Kind#EXIT}. */
    public int exitCode() {
        return exitCode;
    }
}
