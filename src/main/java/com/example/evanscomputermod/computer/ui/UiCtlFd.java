package com.example.evanscomputermod.computer.ui;

import com.example.evanscomputermod.computer.wasi.DeviceFd;

import java.nio.charset.StandardCharsets;

/**
 * {@code /dev/uictl}: the control device behind the {@code gui} command
 * (ROADMAP P2 "Modes"). Write one command line, then read its answer:
 * <pre>
 *   start              start the compositor and its shell if needed, show the GUI
 *   console | show     show the console / the GUI (both keep running)
 *   toggle             what Ctrl+Alt+Backspace does
 *   stop               close the GUI (apps lose their connections)
 *   status             one line, then one line per window
 *   launch PROG ARGS   start a program detached, as a GUI app
 * </pre>
 */
public final class UiCtlFd implements DeviceFd {
    private final UiManager mgr;
    private byte[] answer = new byte[0];
    private int pos;
    private boolean nonBlocking;

    public UiCtlFd(UiManager mgr) {
        this.mgr = mgr;
    }

    @Override
    public synchronized int write(byte[] data, int offset, int length) {
        String line = new String(data, offset, length, StandardCharsets.UTF_8).trim();
        answer = mgr.control(line).getBytes(StandardCharsets.UTF_8);
        pos = 0;
        return length;
    }

    @Override
    public synchronized int read(byte[] buf, int offset, int length) {
        int n = Math.min(length, answer.length - pos);
        if (n <= 0) return 0;
        System.arraycopy(answer, pos, buf, offset, n);
        pos += n;
        return n;
    }

    @Override
    public void close() {}

    @Override
    public boolean isReadable() {
        return true;
    }

    @Override
    public boolean isWritable() {
        return true;
    }

    @Override
    public boolean isNonBlocking() {
        return nonBlocking;
    }

    @Override
    public void setNonBlocking(boolean nb) {
        nonBlocking = nb;
    }
}
