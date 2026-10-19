package com.example.evanscomputermod.computer.wasi;

import org.junit.jupiter.api.Test;

import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.List;

import static org.junit.jupiter.api.Assertions.*;

/**
 * The two WASI pieces vim's port leans on: waiting for a key with a timeout
 * ({@code poll_oneoff} on stdin, a {@link PipeFd}) and listing a directory with
 * wasi-libc's readdir ({@code fd_readdir} filling its buffer).
 */
class StdinPollAndReaddirTest {

    @Test
    void stdinIsPollableAndWakesItsWaiters() throws Exception {
        WasiPipe pipe = new WasiPipe(4096);
        PipeFd stdin = new PipeFd(pipe, true).tty();
        PipeFd keyboard = new PipeFd(pipe, false);
        assertFalse(stdin.readReady(), "nothing typed yet");
        assertEquals(0, stdin.readable());

        Object waiter = new Object();
        stdin.addWaiter(waiter);
        boolean[] woke = {false};
        Thread t = new Thread(() -> {
            synchronized (waiter) {
                try {
                    // Without a notify this would sleep the full 5 s.
                    long start = System.nanoTime();
                    while (!stdin.readReady() && System.nanoTime() - start < 5_000_000_000L) waiter.wait(5000);
                } catch (InterruptedException ignored) {
                }
                woke[0] = stdin.readReady();
            }
        });
        t.start();
        Thread.sleep(50);
        long typed = System.nanoTime();
        keyboard.write("\u001b".getBytes(StandardCharsets.UTF_8), 0, 1);
        t.join(5000);
        assertTrue(woke[0], "the waiter saw the key");
        assertTrue(System.nanoTime() - typed < 2_000_000_000L, "woken by the write, not by its timeout");
        assertEquals(1, stdin.readable());
        stdin.removeWaiter(waiter);

        byte[] buf = new byte[8];
        assertEquals(1, stdin.read(buf, 0, 8));
        assertFalse(stdin.readReady(), "read: nothing left");
        // The keyboard going away is readiness too (the read sees the end).
        keyboard.close();
        assertTrue(stdin.readReady());
        assertTrue(stdin.hungUp());
    }

    @Test
    void readdirFillsTheBufferCuttingTheLastEntry() {
        List<DirFd.DirEntry> entries = new ArrayList<>();
        for (int i = 0; i < 300; i++) entries.add(new DirFd.DirEntry("file" + i + ".vim", (byte) 4, i + 1));
        byte[] out = WasiFunctions.direntRecords(entries, 0, 4096);
        assertEquals(4096, out.length, "a full buffer: there is more");

        // Walk the records the way wasi-libc does: whole ones, then the cut one.
        ByteBuffer b = ByteBuffer.wrap(out).order(ByteOrder.LITTLE_ENDIAN);
        int off = 0, whole = 0;
        long lastCookie = 0;
        while (off + 24 <= out.length) {
            int len = b.getInt(off + 16);
            if (off + 24 + len > out.length) break;
            String name = new String(out, off + 24, len, StandardCharsets.UTF_8);
            assertEquals("file" + whole + ".vim", name);
            lastCookie = b.getLong(off);
            off += 24 + len;
            whole++;
        }
        assertTrue(whole > 100 && whole < 300, "whole entries: " + whole);
        assertEquals(whole, lastCookie);

        // The next call goes on from that cookie and ends with a short buffer.
        List<DirFd.DirEntry> rest = entries.subList((int) lastCookie, entries.size());
        byte[] tail = WasiFunctions.direntRecords(rest, lastCookie, 1 << 16);
        assertTrue(tail.length < 1 << 16, "a short buffer: the end of the directory");
        ByteBuffer t = ByteBuffer.wrap(tail).order(ByteOrder.LITTLE_ENDIAN);
        assertEquals(lastCookie + 1, t.getLong(0));
        assertEquals("file" + whole + ".vim", new String(tail, 24, t.getInt(16), StandardCharsets.UTF_8));

        // Control: a directory that fits leaves the buffer short.
        assertTrue(WasiFunctions.direntRecords(entries.subList(0, 3), 0, 4096).length < 4096);
    }
}
