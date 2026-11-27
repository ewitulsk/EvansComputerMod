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

    @Test
    void sharedFdHandleIsPollableAndWakesWaiters() throws Exception {
        WasiPipe pipe = new WasiPipe(1024);
        PipeFd readEnd = new PipeFd(pipe, true);
        PipeFd writeEnd = new PipeFd(pipe, false);
        SharedFd shared = new SharedFd(readEnd);
        SharedFd.Handle handle = shared.open();

        assertFalse(handle.readReady());
        Object waiter = new Object();
        handle.addWaiter(waiter);
        boolean[] woke = {false};
        Thread t = new Thread(() -> {
            synchronized (waiter) {
                try {
                    long start = System.nanoTime();
                    while (!handle.readReady() && System.nanoTime() - start < 5_000_000_000L) waiter.wait(5000);
                } catch (InterruptedException ignored) {}
                woke[0] = handle.readReady();
            }
        });
        t.start();
        Thread.sleep(30);
        writeEnd.write("x".getBytes(StandardCharsets.UTF_8), 0, 1);
        t.join(5000);
        assertTrue(woke[0], "SharedFd.Handle woke poll waiter");
        assertEquals(1, handle.readable());
        handle.removeWaiter(waiter);
        handle.close();
        assertTrue(handle.hungUp());
    }

    @Test
    void wasiPathOpenExclDirectoryFdstatAndUnlinkSemantics() throws Exception {
        var dir = java.nio.file.Files.createTempDirectory("ecm-wasi-vim-test");
        try {
            var sub = java.nio.file.Files.createDirectory(dir.resolve("sub"));
            var existing = dir.resolve("a.txt");
            java.nio.file.Files.writeString(existing, "hello");

            FdTable fds = new FdTable();
            WasiPipe kbd = new WasiPipe(64);
            fds.insertAt(0, new PipeFd(kbd, true).tty());
            WasiFunctions.WasiState state = new WasiFunctions.WasiState(fds, new String[]{"vim"}, dir, java.util.Map.of());
            state.keyboard = kbd;
            FakeMem mem = new FakeMem(4096);
            state.instance = new FakeInst(mem);

            List<com.example.evanscomputermod.api.wasm.WasmHostFunc> funcs = new ArrayList<>();
            WasiFunctions.register(state, funcs, null);
            java.util.Map<String, com.example.evanscomputermod.api.wasm.WasmHostFunc.Handler> byName = new java.util.HashMap<>();
            for (var f : funcs) {
                if ("wasi_snapshot_preview1".equals(f.moduleName())) byName.put(f.fieldName(), f.handler());
            }
            java.util.function.BiFunction<String, long[], Long> call = (name, a) ->
                    byName.get(name).invoke(state.instance, a)[0];

            // O_CREAT | O_EXCL on existing file -> EEXIST (20).
            mem.writeString(0, "a.txt");
            assertEquals(20L, call.apply("path_open", new long[]{3, 0, 0, 5, 1 | 4, 1L << 6, 0, 0, 100}));

            // O_CREAT | O_EXCL on new file -> SUCCESS (0), second time -> EEXIST (20).
            mem.writeString(0, "new.swp");
            assertEquals(0L, call.apply("path_open", new long[]{3, 0, 0, 7, 1 | 4, 1L << 6, 0, 0, 100}));
            int swpFd = mem.readInt(100);
            assertEquals(20L, call.apply("path_open", new long[]{3, 0, 0, 7, 1 | 4, 1L << 6, 0, 0, 100}));
            fds.close(swpFd);

            // O_DIRECTORY on regular file -> ENOTDIR (54); write on directory -> EISDIR (31).
            mem.writeString(0, "a.txt");
            assertEquals(54L, call.apply("path_open", new long[]{3, 0, 0, 5, 2, 0, 0, 0, 100}));
            mem.writeString(0, "sub");
            assertEquals(31L, call.apply("path_open", new long[]{3, 0, 0, 3, 0, 1L << 6, 0, 0, 100}));

            // Open directory read-only -> fd_fdstat_get reports DIRECTORY (3).
            assertEquals(0L, call.apply("path_open", new long[]{3, 0, 0, 3, 2, 0, 0, 0, 100}));
            int dirFd = mem.readInt(100);
            assertEquals(0L, call.apply("fd_fdstat_get", new long[]{dirFd, 200}));
            assertEquals(3, mem.readByte(200), "opened DirFd has FILETYPE_DIRECTORY");

            // /dev/tty opened at fd >= 4 -> fd_fdstat_get reports CHARACTER_DEVICE (2).
            mem.writeString(0, "dev/tty");
            assertEquals(0L, call.apply("path_open", new long[]{3, 0, 0, 7, 0, 0, 0, 0, 100}));
            int ttyFd = mem.readInt(100);
            assertTrue(ttyFd >= 4);
            assertEquals(0L, call.apply("fd_fdstat_get", new long[]{ttyFd, 200}));
            assertEquals(2, mem.readByte(200), "/dev/tty has FILETYPE_CHARACTER_DEVICE");

            // Invalid fd -> EBADF (8) for both fd_fdstat_get and fd_filestat_get.
            assertEquals(8L, call.apply("fd_fdstat_get", new long[]{999, 200}));
            assertEquals(8L, call.apply("fd_filestat_get", new long[]{999, 200}));

            // path_unlink_file: ENOENT (44) on missing, EISDIR (31) on dir, traversal rejected, SUCCESS on file.
            mem.writeString(0, "missing.txt");
            assertEquals(44L, call.apply("path_unlink_file", new long[]{3, 0, 11}));
            mem.writeString(0, "sub");
            assertEquals(31L, call.apply("path_unlink_file", new long[]{3, 0, 3}));
            mem.writeString(0, "../escape");
            assertEquals(44L, call.apply("path_unlink_file", new long[]{3, 0, 9}));
            assertEquals(44L, call.apply("path_create_directory", new long[]{3, 0, 9}));
            mem.writeString(0, "new.swp");
            assertEquals(0L, call.apply("path_unlink_file", new long[]{3, 0, 7}));
            assertFalse(java.nio.file.Files.exists(dir.resolve("new.swp")));

            // DirFd inode matches WasiFunctions.writeFilestat inode and is non-zero.
            DirFd rootDir = new DirFd(dir);
            var aEntry = rootDir.readDir(0).stream().filter(e -> e.name().equals("a.txt")).findFirst().orElseThrow();
            WasiFunctions.writeFilestat(mem, 300, existing);
            assertEquals(mem.readLong(308), aEntry.inode());
            assertNotEquals(0L, aEntry.inode());

            fds.closeAll();
        } finally {
            try (var s = java.nio.file.Files.walk(dir)) {
                s.sorted(java.util.Comparator.reverseOrder()).forEach(p -> p.toFile().delete());
            }
        }
    }

    private static final class FakeMem implements com.example.evanscomputermod.api.wasm.WasmMemory {
        private final byte[] b;
        private final ByteBuffer bb;
        FakeMem(int size) { this.b = new byte[size]; this.bb = ByteBuffer.wrap(b).order(ByteOrder.LITTLE_ENDIAN); }
        @Override public int size() { return b.length; }
        @Override public int pages() { return b.length / 65536; }
        @Override public int grow(int deltaPages) { return -1; }
        @Override public byte readByte(int addr) { return b[addr]; }
        @Override public void writeByte(int addr, byte v) { b[addr] = v; }
        @Override public short readShort(int addr) { return bb.getShort(addr); }
        @Override public void writeShort(int addr, short v) { bb.putShort(addr, v); }
        @Override public int readInt(int addr) { return bb.getInt(addr); }
        @Override public void writeInt(int addr, int v) { bb.putInt(addr, v); }
        @Override public long readLong(int addr) { return bb.getLong(addr); }
        @Override public void writeLong(int addr, long v) { bb.putLong(addr, v); }
        @Override public float readFloat(int addr) { return bb.getFloat(addr); }
        @Override public void writeFloat(int addr, float v) { bb.putFloat(addr, v); }
        @Override public double readDouble(int addr) { return bb.getDouble(addr); }
        @Override public void writeDouble(int addr, double v) { bb.putDouble(addr, v); }
        @Override public byte[] readBytes(int addr, int len) { byte[] out = new byte[len]; System.arraycopy(b, addr, out, 0, len); return out; }
        @Override public void writeBytes(int addr, byte[] src, int srcOff, int len) { System.arraycopy(src, srcOff, b, addr, len); }
    }

    private static final class FakeInst implements com.example.evanscomputermod.api.wasm.WasmInstance {
        private final com.example.evanscomputermod.api.wasm.WasmMemory mem;
        FakeInst(com.example.evanscomputermod.api.wasm.WasmMemory mem) { this.mem = mem; }
        @Override public boolean hasExport(String name) { return false; }
        @Override public long[] callExport(String name, long... args) { return new long[0]; }
        @Override public com.example.evanscomputermod.api.wasm.WasmExport export(String name) { return null; }
        @Override public com.example.evanscomputermod.api.wasm.WasmMemory memory() { return mem; }
        @Override public void requestInterrupt() {}
        @Override public void clearInterrupt() {}
        @Override public void close() {}
    }
}
