package com.example.evanscomputermod.computer.ui;

import com.example.evanscomputermod.computer.wasi.Pollable;
import com.example.evanscomputermod.computer.wasi.WasiFileDescriptor;

/**
 * The UI file descriptor a program gets from {@code ui_open} (§3). Its data
 * moves through {@code ui_send} / {@code ui_recv}, not {@code fd_read} /
 * {@code fd_write}; it works with {@code poll_oneoff} and {@code sock_poll}.
 * Closing it (or the program exiting) closes the connection.
 */
public final class UiFd implements WasiFileDescriptor, Pollable {
    private final UiConnection conn;

    public UiFd(UiConnection conn) {
        this.conn = conn;
    }

    public UiConnection connection() {
        return conn;
    }

    @Override
    public int read(byte[] buf, int offset, int length) {
        return -1;
    }

    @Override
    public int write(byte[] data, int offset, int length) {
        return -1;
    }

    @Override
    public void close() {
        conn.close();
    }

    @Override
    public boolean isReadable() {
        return true;
    }

    @Override
    public boolean isWritable() {
        return true;
    }

    @Override
    public boolean readReady() {
        return conn.readReady();
    }

    @Override
    public int readable() {
        return conn.readable();
    }

    @Override
    public boolean hungUp() {
        return conn.isClosed();
    }

    @Override
    public void addWaiter(Object waiter) {
        conn.addWaiter(waiter);
    }

    @Override
    public void removeWaiter(Object waiter) {
        conn.removeWaiter(waiter);
    }
}
