package com.example.evanscomputermod.computer.wasi;

import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.api.wasm.WasmHostFunc;
import com.example.evanscomputermod.api.wasm.WasmInstance;
import com.example.evanscomputermod.api.wasm.WasmMemory;
import com.example.evanscomputermod.api.wasm.WasmTrap;
import com.example.evanscomputermod.api.wasm.WasmValType;

import java.io.IOException;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.List;

import static com.example.evanscomputermod.api.wasm.WasmHostFunc.retI32;
import static com.example.evanscomputermod.api.wasm.WasmHostFunc.retI64;

/**
 * Provides WASI snapshot_preview1 and {@code env::*} host functions for
 * child WASM processes. Each child process gets its own {@link WasiState}
 * holding its FdTable, argv, env, and storage path; the registration
 * methods here build a list of {@link WasmHostFunc} entries that close over
 * that state.
 */
public class WasiFunctions {

    private static final String WASI_NS = "wasi_snapshot_preview1";
    private static final String ENV_NS = "env";

    private static final int ERRNO_SUCCESS = 0;
    private static final int ERRNO_AGAIN = 6;
    private static final int FDFLAGS_NONBLOCK = 4;
    private static final int ERRNO_BADF = 8;
    private static final int ERRNO_INVAL = 28;
    private static final int ERRNO_NOSYS = 52;
    private static final int ERRNO_NOENT = 44;

    private static final List<WasmValType> I32 = List.of(WasmValType.I32);
    private static final List<WasmValType> I32_I32 = List.of(WasmValType.I32, WasmValType.I32);
    private static final List<WasmValType> I32_I32_I32 = List.of(WasmValType.I32, WasmValType.I32, WasmValType.I32);
    private static final List<WasmValType> I32x4 = List.of(WasmValType.I32, WasmValType.I32, WasmValType.I32, WasmValType.I32);
    private static final List<WasmValType> I32x5 = List.of(WasmValType.I32, WasmValType.I32, WasmValType.I32, WasmValType.I32, WasmValType.I32);
    private static final List<WasmValType> I32x6 = List.of(WasmValType.I32, WasmValType.I32, WasmValType.I32, WasmValType.I32, WasmValType.I32, WasmValType.I32);
    private static final List<WasmValType> I32x8 = List.of(WasmValType.I32, WasmValType.I32, WasmValType.I32, WasmValType.I32, WasmValType.I32, WasmValType.I32, WasmValType.I32, WasmValType.I32);
    private static final List<WasmValType> RET_I32 = List.of(WasmValType.I32);
    private static final List<WasmValType> RET_I64 = List.of(WasmValType.I64);
    private static final List<WasmValType> RET_NONE = List.of();
    private static final List<WasmValType> NO_PARAMS = List.of();

    /**
     * Mutable state held by each child process. Owned by ProcessManager and
     * referenced from every WASI host-function closure.
     */
    public static class WasiState {
        public final FdTable fdTable;
        public final String[] argv;
        public final Path storagePath;
        public final java.util.Map<String, String> envVars;
        /** Set by ProcessManager once the child instance is built. */
        public WasmInstance instance;
        /** This program's peripheral event queue; created on its first peripheral call. */
        public com.example.evanscomputermod.computer.peripheral.PeripheralEventBus.Subscriber peripheralEvents;
        /** A peripheral result too large for the program's buffer, kept for periph_take_pending. */
        byte[] pendingPeripheralResult;

        public WasiState(FdTable fdTable, String[] argv, Path storagePath,
                         java.util.Map<String, String> envVars) {
            this.fdTable = fdTable;
            this.argv = argv;
            this.storagePath = storagePath;
            this.envVars = envVars != null ? envVars : java.util.Map.of();
        }

        WasmMemory mem() {
            return instance.memory();
        }
    }

    /**
     * Register all snapshot_preview1 host functions. Each entry is added
     * twice — once with the {@code wasi_snapshot_preview1} module name, once
     * unqualified — so import resolution finds the function regardless of
     * how the guest module declared it.
     */
    public static void register(WasiState state, List<WasmHostFunc> sink, ChildHostBridge childBridge) {

        addWasi(sink, "fd_write", I32x4, RET_I32, (inst, args) -> {
            int fd = (int) args[0];
            int iovsPtr = (int) args[1];
            int iovsLen = (int) args[2];
            int nwrittenPtr = (int) args[3];
            return retI32(wasifdWrite(state, fd, iovsPtr, iovsLen, nwrittenPtr));
        });

        addWasi(sink, "fd_read", I32x4, RET_I32, (inst, args) -> {
            int fd = (int) args[0];
            int iovsPtr = (int) args[1];
            int iovsLen = (int) args[2];
            int nreadPtr = (int) args[3];
            return retI32(wasifdRead(state, fd, iovsPtr, iovsLen, nreadPtr));
        });

        addWasi(sink, "fd_close", I32, RET_I32, (inst, args) -> {
            int fd = (int) args[0];
            WasiFileDescriptor desc = state.fdTable.get(fd);
            if (fd > 2) {
                EvansComputerMod.LOGGER.debug("WASI fd_close: fd={} ({})", fd,
                        desc != null ? desc.getClass().getSimpleName() : "null");
            }
            state.fdTable.close(fd);
            return retI32(ERRNO_SUCCESS);
        });

        addWasi(sink, "fd_seek",
                List.of(WasmValType.I32, WasmValType.I64, WasmValType.I32, WasmValType.I32),
                RET_I32, (inst, args) -> {
            int fd = (int) args[0];
            WasiFileDescriptor desc = state.fdTable.get(fd);
            if (desc instanceof VfsFileFd vfs) {
                try {
                    long newOff = vfs.seek(args[1], (int) args[2]);
                    state.mem().writeLong((int) args[3], newOff);
                    return retI32(ERRNO_SUCCESS);
                } catch (IOException e) {
                    return retI32(ERRNO_INVAL);
                }
            }
            return retI32(ERRNO_NOSYS);
        });

        addWasi(sink, "fd_fdstat_get", I32_I32, RET_I32, (inst, args) -> {
            int fd = (int) args[0];
            int bufPtr = (int) args[1];
            WasmMemory mem = state.mem();
            // fdstat: filetype(1), fdflags(2), rights_base(8), rights_inheriting(8) = 24 bytes
            for (int i = 0; i < 24; i++) mem.writeByte(bufPtr + i, (byte) 0);
            WasiFileDescriptor statDesc = state.fdTable.get(fd);
            if ((statDesc instanceof PipeFd p && p.isNonBlocking())
                    || (statDesc instanceof DeviceFd d && d.isNonBlocking())) {
                mem.writeShort(bufPtr + 2, (short) FDFLAGS_NONBLOCK);
            }
            if (fd <= 2 || statDesc instanceof DeviceFd) {
                mem.writeByte(bufPtr, (byte) 2);  // FILETYPE_CHARACTER_DEVICE
            } else if (fd == 3) {
                mem.writeByte(bufPtr, (byte) 3);  // FILETYPE_DIRECTORY
            } else {
                mem.writeByte(bufPtr, (byte) 4);  // FILETYPE_REGULAR_FILE
            }
            mem.writeLong(bufPtr + 8, -1L);
            mem.writeLong(bufPtr + 16, -1L);
            return retI32(ERRNO_SUCCESS);
        });

        addWasi(sink, "fd_fdstat_set_flags", I32_I32, RET_I32, (inst, args) -> {
            // Only O_NONBLOCK on pipe ends (e.g. stdin) has an effect: it lets
            // interactive programs like ssh poll the keyboard between socket reads.
            WasiFileDescriptor flagDesc = state.fdTable.get((int) args[0]);
            if (flagDesc instanceof PipeFd p) {
                p.setNonBlocking(((int) args[1] & FDFLAGS_NONBLOCK) != 0);
            } else if (flagDesc instanceof DeviceFd d) {
                d.setNonBlocking(((int) args[1] & FDFLAGS_NONBLOCK) != 0);
            }
            return retI32(ERRNO_SUCCESS);
        });

        addWasi(sink, "fd_filestat_get", I32_I32, RET_I32, (inst, args) -> {
            int fd = (int) args[0];
            int bufPtr = (int) args[1];
            WasmMemory mem = state.mem();
            for (int i = 0; i < 64; i++) mem.writeByte(bufPtr + i, (byte) 0);
            WasiFileDescriptor desc = state.fdTable.get(fd);
            if (desc instanceof VfsFileFd vfs) {
                mem.writeByte(bufPtr + 16, (byte) 4);
                try { mem.writeLong(bufPtr + 32, vfs.size()); } catch (IOException ignored) {}
            } else if (desc instanceof DirFd) {
                mem.writeByte(bufPtr + 16, (byte) 3);
            } else {
                mem.writeByte(bufPtr + 16, (byte) 2);
            }
            return retI32(ERRNO_SUCCESS);
        });

        addWasi(sink, "fd_prestat_get", I32_I32, RET_I32, (inst, args) -> {
            int fd = (int) args[0];
            if (fd == 3) {
                WasmMemory mem = state.mem();
                int bufPtr = (int) args[1];
                mem.writeByte(bufPtr, (byte) 0);
                mem.writeInt(bufPtr + 4, 1);
                return retI32(ERRNO_SUCCESS);
            }
            return retI32(ERRNO_BADF);
        });

        addWasi(sink, "fd_prestat_dir_name", I32_I32_I32, RET_I32, (inst, args) -> {
            if ((int) args[0] == 3) {
                state.mem().writeByte((int) args[1], (byte) '/');
                return retI32(ERRNO_SUCCESS);
            }
            return retI32(ERRNO_BADF);
        });

        addWasi(sink, "proc_exit", I32, RET_NONE, (inst, args) -> {
            throw new WasmTrap((int) args[0]);
        });

        addWasi(sink, "args_sizes_get", I32_I32, RET_I32, (inst, args) -> {
            WasmMemory mem = state.mem();
            String[] argv = state.argv;
            int totalSize = 0;
            for (String arg : argv) totalSize += arg.getBytes(StandardCharsets.UTF_8).length + 1;
            mem.writeInt((int) args[0], argv.length);
            mem.writeInt((int) args[1], totalSize);
            return retI32(ERRNO_SUCCESS);
        });

        addWasi(sink, "args_get", I32_I32, RET_I32, (inst, args) -> {
            WasmMemory mem = state.mem();
            String[] argv = state.argv;
            int argvPtr = (int) args[0];
            int bufPtr = (int) args[1];
            for (int i = 0; i < argv.length; i++) {
                mem.writeInt(argvPtr + i * 4, bufPtr);
                byte[] bytes = argv[i].getBytes(StandardCharsets.UTF_8);
                for (byte b : bytes) mem.writeByte(bufPtr++, b);
                mem.writeByte(bufPtr++, (byte) 0);
            }
            return retI32(ERRNO_SUCCESS);
        });

        addWasi(sink, "environ_sizes_get", I32_I32, RET_I32, (inst, args) -> {
            WasmMemory mem = state.mem();
            var env = state.envVars;
            int count = env.size();
            int bufSize = 0;
            for (var e : env.entrySet()) {
                bufSize += e.getKey().length() + 1 + e.getValue().length() + 1;
            }
            mem.writeInt((int) args[0], count);
            mem.writeInt((int) args[1], bufSize);
            return retI32(ERRNO_SUCCESS);
        });

        addWasi(sink, "environ_get", I32_I32, RET_I32, (inst, args) -> {
            WasmMemory mem = state.mem();
            int environPtr = (int) args[0];
            int bufPtr = (int) args[1];
            int offset = 0;
            int idx = 0;
            for (var e : state.envVars.entrySet()) {
                mem.writeInt(environPtr + idx * 4, bufPtr + offset);
                byte[] entry = (e.getKey() + "=" + e.getValue()).getBytes(StandardCharsets.UTF_8);
                for (byte b : entry) {
                    mem.writeByte(bufPtr + offset, b);
                    offset++;
                }
                mem.writeByte(bufPtr + offset, (byte) 0);
                offset++;
                idx++;
            }
            return retI32(ERRNO_SUCCESS);
        });

        addWasi(sink, "clock_time_get",
                List.of(WasmValType.I32, WasmValType.I64, WasmValType.I32),
                RET_I32, (inst, args) -> {
            long nanos = clockNow((int) args[0]);
            if (nanos < 0) return retI32(ERRNO_INVAL);
            state.mem().writeLong((int) args[2], nanos);
            return retI32(ERRNO_SUCCESS);
        });

        addWasi(sink, "clock_res_get", I32_I32, RET_I32, (inst, args) -> {
            int id = (int) args[0];
            if (id < CLOCK_REALTIME || id > CLOCK_THREAD_CPUTIME) return retI32(ERRNO_INVAL);
            state.mem().writeLong((int) args[1], 1_000L); // 1 µs
            return retI32(ERRNO_SUCCESS);
        });

        addWasi(sink, "random_get", I32_I32, RET_I32, (inst, args) -> {
            WasmMemory mem = state.mem();
            int ptr = (int) args[0];
            int len = (int) args[1];
            byte[] bytes = new byte[len];
            com.example.evanscomputermod.computer.wasi.Entropy.fill(bytes);
            mem.writeBytes(ptr, bytes);
            return retI32(ERRNO_SUCCESS);
        });

        addWasi(sink, "sched_yield", NO_PARAMS, RET_I32, (inst, args) -> {
            Thread.yield();
            return retI32(ERRNO_SUCCESS);
        });

        addWasi(sink, "path_open",
                List.of(WasmValType.I32, WasmValType.I32, WasmValType.I32, WasmValType.I32,
                        WasmValType.I32, WasmValType.I64, WasmValType.I64, WasmValType.I32, WasmValType.I32),
                RET_I32, (inst, argsArr) -> {
            int pathPtr = (int) argsArr[2];
            int pathLen = (int) argsArr[3];
            int oflags = (int) argsArr[4];
            int fdflags = (int) argsArr[7];
            int fdOutPtr = (int) argsArr[8];

            WasmMemory mem = state.mem();
            String pathStr = mem.readString(pathPtr, pathLen);

            if (pathStr.contains("..") || pathStr.startsWith("/")) {
                EvansComputerMod.LOGGER.debug("WASI path_open: rejected path (traversal): {}", pathStr);
                return retI32(ERRNO_NOENT);
            }

            // /dev: devices of the computer (speakers), not files.
            if (pathStr.startsWith("dev/")) {
                WasiFileDescriptor dev = childBridge == null ? null : childBridge.openDevice(pathStr.substring(4));
                if (dev == null) return retI32(ERRNO_NOENT);
                mem.writeInt(fdOutPtr, state.fdTable.allocate(dev));
                return retI32(ERRNO_SUCCESS);
            }

            Path filePath = state.storagePath.resolve(pathStr).normalize();
            if (!filePath.startsWith(state.storagePath)) {
                EvansComputerMod.LOGGER.debug("WASI path_open: rejected path (escape): {}", pathStr);
                return retI32(ERRNO_NOENT);
            }

            boolean create = (oflags & 1) != 0;
            boolean trunc = (oflags & 8) != 0;
            boolean append = (fdflags & 1) != 0;

            EvansComputerMod.LOGGER.debug("WASI path_open: path='{}' oflags={} create={} trunc={} append={} resolved={}",
                    pathStr, oflags, create, trunc, append, filePath);

            if (!Files.exists(filePath) && !create) {
                EvansComputerMod.LOGGER.debug("WASI path_open: NOENT (no O_CREAT)");
                return retI32(ERRNO_NOENT);
            }

            try {
                if (Files.isDirectory(filePath)) {
                    DirFd dirFd = new DirFd(filePath);
                    int newFd = state.fdTable.allocate(dirFd);
                    mem.writeInt(fdOutPtr, newFd);
                    EvansComputerMod.LOGGER.debug("WASI path_open: opened dir fd={}", newFd);
                } else {
                    if (create && !Files.exists(filePath)) {
                        Files.createDirectories(filePath.getParent());
                        Files.createFile(filePath);
                        EvansComputerMod.LOGGER.debug("WASI path_open: created new file");
                    }
                    VfsFileFd vfs = new VfsFileFd(filePath, true, true, append);
                    if (trunc) vfs.truncate();
                    int newFd = state.fdTable.allocate(vfs);
                    mem.writeInt(fdOutPtr, newFd);
                    EvansComputerMod.LOGGER.debug("WASI path_open: opened file fd={} size={}", newFd, Files.size(filePath));
                }
                return retI32(ERRNO_SUCCESS);
            } catch (IOException e) {
                EvansComputerMod.LOGGER.error("WASI path_open: IOException for {}", pathStr, e);
                return retI32(ERRNO_NOENT);
            }
        });

        addWasi(sink, "path_create_directory", I32_I32_I32, RET_I32, (inst, args) -> {
            String pathStr = state.mem().readString((int) args[1], (int) args[2]);
            try {
                Path p = state.storagePath.resolve(pathStr).normalize();
                Files.createDirectories(p);
                return retI32(ERRNO_SUCCESS);
            } catch (IOException e) {
                return retI32(ERRNO_NOENT);
            }
        });

        addWasi(sink, "path_remove_directory", I32_I32_I32, RET_I32, (inst, args) -> retI32(ERRNO_NOSYS));

        addWasi(sink, "path_unlink_file", I32_I32_I32, RET_I32, (inst, args) -> {
            String pathStr = state.mem().readString((int) args[1], (int) args[2]);
            try {
                Path p = state.storagePath.resolve(pathStr).normalize();
                Files.deleteIfExists(p);
                return retI32(ERRNO_SUCCESS);
            } catch (IOException e) {
                return retI32(ERRNO_NOENT);
            }
        });

        addWasi(sink, "path_filestat_get", I32x5, RET_I32, (inst, args) -> {
            String pathStr = state.mem().readString((int) args[2], (int) args[3]);
            if (pathStr.startsWith("dev/") && childBridge != null && childBridge.hasDevice(pathStr.substring(4))) {
                WasmMemory dm = state.mem();
                for (int i = 0; i < 64; i++) dm.writeByte((int) args[4] + i, (byte) 0);
                dm.writeByte((int) args[4] + 16, (byte) 2); // character device
                return retI32(ERRNO_SUCCESS);
            }
            Path p = state.storagePath.resolve(pathStr).normalize();
            if (!Files.exists(p)) {
                return retI32(ERRNO_NOENT);
            }
            WasmMemory mem = state.mem();
            int bufPtr = (int) args[4];
            for (int i = 0; i < 64; i++) mem.writeByte(bufPtr + i, (byte) 0);
            try {
                byte filetype = Files.isDirectory(p) ? (byte) 3 : (byte) 4;
                mem.writeByte(bufPtr + 16, filetype);
                mem.writeLong(bufPtr + 32, Files.size(p));
            } catch (IOException ignored) {}
            return retI32(ERRNO_SUCCESS);
        });

        addWasi(sink, "fd_readdir",
                List.of(WasmValType.I32, WasmValType.I32, WasmValType.I32, WasmValType.I64, WasmValType.I32),
                RET_I32, (inst, args) -> {
            int fd = (int) args[0];
            int bufPtr = (int) args[1];
            int bufLen = (int) args[2];
            long cookie = args[3];
            int bufusedPtr = (int) args[4];

            WasiFileDescriptor desc = state.fdTable.get(fd);
            if (!(desc instanceof DirFd dirFd)) return retI32(ERRNO_BADF);

            WasmMemory mem = state.mem();

            java.util.List<DirFd.DirEntry> entries = dirFd.readDir(cookie);
            int offset = 0;
            long nextCookie = cookie;

            for (DirFd.DirEntry entry : entries) {
                nextCookie++;
                byte[] nameBytes = entry.name().getBytes(StandardCharsets.UTF_8);
                int entrySize = 24 + nameBytes.length;
                if (offset + entrySize > bufLen) break;
                int pos = bufPtr + offset;
                mem.writeLong(pos, nextCookie);
                mem.writeLong(pos + 8, entry.inode());
                mem.writeInt(pos + 16, nameBytes.length);
                mem.writeByte(pos + 20, entry.type());
                mem.writeBytes(pos + 24, nameBytes);
                offset += entrySize;
            }

            mem.writeInt(bufusedPtr, offset);
            return retI32(ERRNO_SUCCESS);
        });

        // === Kernel-style file_* host functions (used by ecm_host_abi::fs) ===

        addEnv(sink, "file_read", I32x4, RET_I32, (inst, args) -> {
            String path = state.mem().readString((int) args[0], (int) args[1]);
            int bufPtr = (int) args[2];
            int bufLen = (int) args[3];
            Path filePath = resolveChildPath(state, path);
            if (filePath == null || !Files.exists(filePath) || Files.isDirectory(filePath)) {
                return retI32(-1);
            }
            try {
                byte[] data = Files.readAllBytes(filePath);
                int n = Math.min(data.length, bufLen);
                state.mem().writeBytes(bufPtr, data, 0, n);
                return retI32(n);
            } catch (IOException e) {
                return retI32(-1);
            }
        });

        addEnv(sink, "file_write", I32x4, RET_I32, (inst, args) -> {
            String path = state.mem().readString((int) args[0], (int) args[1]);
            int dataPtr = (int) args[2];
            int dataLen = (int) args[3];
            Path filePath = resolveChildPath(state, path);
            if (filePath == null || dataLen < 0 || dataLen > 1024 * 1024) {
                return retI32(-1);
            }
            try {
                byte[] data = state.mem().readBytes(dataPtr, dataLen);
                Path parent = filePath.getParent();
                if (parent != null && !Files.exists(parent)) {
                    Files.createDirectories(parent);
                }
                Files.write(filePath, data, java.nio.file.StandardOpenOption.CREATE,
                        java.nio.file.StandardOpenOption.TRUNCATE_EXISTING);
                return retI32(dataLen);
            } catch (IOException e) {
                return retI32(-1);
            }
        });

        addEnv(sink, "file_size", I32_I32, RET_I32, (inst, args) -> {
            String path = state.mem().readString((int) args[0], (int) args[1]);
            Path filePath = resolveChildPath(state, path);
            if (filePath == null || !Files.exists(filePath)) return retI32(-1);
            try {
                return retI32((int) Files.size(filePath));
            } catch (IOException e) {
                return retI32(-1);
            }
        });

        addEnv(sink, "file_exists", I32_I32, RET_I32, (inst, args) -> {
            String path = state.mem().readString((int) args[0], (int) args[1]);
            Path filePath = resolveChildPath(state, path);
            return retI32(filePath != null && Files.exists(filePath) ? 1 : 0);
        });

        addEnv(sink, "file_delete", I32_I32, RET_I32, (inst, args) -> {
            String path = state.mem().readString((int) args[0], (int) args[1]);
            Path filePath = resolveChildPath(state, path);
            if (filePath == null || !Files.exists(filePath)) return retI32(0);
            try {
                Files.delete(filePath);
                return retI32(1);
            } catch (IOException e) {
                return retI32(0);
            }
        });

        addEnv(sink, "file_mkdir", I32_I32, RET_I32, (inst, args) -> {
            String path = state.mem().readString((int) args[0], (int) args[1]);
            Path dirPath = resolveChildPath(state, path);
            if (dirPath == null) return retI32(-1);
            try {
                Files.createDirectories(dirPath);
                return retI32(0);
            } catch (IOException e) {
                return retI32(-1);
            }
        });

        addEnv(sink, "file_is_dir", I32_I32, RET_I32, (inst, args) -> {
            String path = state.mem().readString((int) args[0], (int) args[1]);
            if (path == null || path.isEmpty()) return retI32(1);
            Path filePath = resolveChildPath(state, path);
            return retI32(filePath != null && Files.isDirectory(filePath) ? 1 : 0);
        });

        addEnv(sink, "file_list", I32_I32, RET_I32, (inst, args) -> {
            int bufPtr = (int) args[0];
            int bufLen = (int) args[1];
            Path root = state.storagePath;
            if (!Files.exists(root)) return retI32(0);
            try (var stream = Files.list(root)) {
                StringBuilder sb = new StringBuilder();
                boolean first = true;
                for (var p : (Iterable<Path>) stream::iterator) {
                    if (!Files.isRegularFile(p)) continue;
                    if (!first) sb.append('\n');
                    sb.append(p.getFileName().toString());
                    first = false;
                }
                byte[] data = sb.toString().getBytes(StandardCharsets.UTF_8);
                int n = Math.min(data.length, bufLen);
                state.mem().writeBytes(bufPtr, data, 0, n);
                return retI32(n);
            } catch (IOException e) {
                return retI32(-1);
            }
        });

        addEnv(sink, "file_list_dir", I32x4, RET_I32, (inst, args) -> {
            String path = state.mem().readString((int) args[0], (int) args[1]);
            int bufPtr = (int) args[2];
            int bufLen = (int) args[3];
            Path dirPath = (path == null || path.isEmpty())
                    ? state.storagePath
                    : resolveChildPath(state, path);
            if (dirPath == null || !Files.isDirectory(dirPath)) return retI32(-1);
            try (var stream = Files.list(dirPath)) {
                StringBuilder sb = new StringBuilder();
                boolean first = true;
                for (var p : (Iterable<Path>) stream::iterator) {
                    if (!first) sb.append('\n');
                    sb.append(Files.isDirectory(p) ? "d:" : "f:");
                    sb.append(p.getFileName().toString());
                    first = false;
                }
                byte[] data = sb.toString().getBytes(StandardCharsets.UTF_8);
                int n = Math.min(data.length, bufLen);
                state.mem().writeBytes(bufPtr, data, 0, n);
                return retI32(n);
            } catch (IOException e) {
                return retI32(-1);
            }
        });

        // === Redstone host functions (delegated through ChildHostBridge) ===

        addEnv(sink, "redstone_set_output", I32_I32, RET_I32, (inst, args) -> {
            if (childBridge == null) return retI32(-1);
            return retI32(childBridge.redstoneSetOutput((int) args[0], (int) args[1]));
        });

        addEnv(sink, "redstone_get_input", I32, RET_I32, (inst, args) -> {
            if (childBridge == null) return retI32(0);
            return retI32(childBridge.redstoneGetInput((int) args[0]));
        });

        addEnv(sink, "redstone_get_all_input", I32, RET_I32, (inst, args) -> {
            if (childBridge == null) return retI32(-1);
            int bufPtr = (int) args[0];
            int[] vals = new int[6];
            int rc = childBridge.redstoneGetAllInput(vals);
            if (rc != 0) return retI32(rc);
            WasmMemory mem = state.mem();
            for (int i = 0; i < 6; i++) {
                mem.writeInt(bufPtr + i * 4, vals[i]);
            }
            return retI32(0);
        });

        // === Peripherals (ecm_host_abi::peripheral) ===
        //
        // Every call returns the length of its encoded result frame. When that
        // exceeds `cap` nothing is written; the frame is kept and the program
        // fetches it with periph_take_pending into a big enough buffer.
        // -1 means the bridge is unavailable.

        addEnv(sink, "periph_list", I32_I32, RET_I32, (inst, args) -> {
            if (childBridge == null) return retI32(-1);
            subscribePeripheralEvents(state, childBridge);
            return retI32(peripheralResult(state, childBridge.peripheralList(), (int) args[0], (int) args[1]));
        });

        addEnv(sink, "periph_methods", I32x4, RET_I32, (inst, args) -> {
            if (childBridge == null) return retI32(-1);
            subscribePeripheralEvents(state, childBridge);
            String name = state.mem().readString((int) args[0], (int) args[1]);
            return retI32(peripheralResult(state, childBridge.peripheralMethods(name), (int) args[2], (int) args[3]));
        });

        addEnv(sink, "periph_call", I32x8, RET_I32, (inst, args) -> {
            if (childBridge == null) return retI32(-1);
            subscribePeripheralEvents(state, childBridge);
            WasmMemory mem = state.mem();
            String name = mem.readString((int) args[0], (int) args[1]);
            String method = mem.readString((int) args[2], (int) args[3]);
            int argsLen = (int) args[5];
            byte[] encoded = argsLen > 0 ? mem.readBytes((int) args[4], argsLen) : new byte[0];
            return retI32(peripheralResult(state, childBridge.peripheralCall(name, method, encoded),
                    (int) args[6], (int) args[7]));
        });

        // (filter, filter_len, timeout_ms, buf, cap) -> frame length, or 0 on timeout.
        // filter_len 0 = any event; timeout_ms < 0 = wait forever.
        addEnv(sink, "periph_wait_event", I32x5, RET_I32, (inst, args) -> {
            if (childBridge == null) return retI32(-1);
            subscribePeripheralEvents(state, childBridge);
            int filterLen = (int) args[1];
            String filter = filterLen > 0 ? state.mem().readString((int) args[0], filterLen) : null;
            byte[] frame = childBridge.peripheralWaitEvent(state.peripheralEvents, filter, (int) args[2]);
            if (frame == null) return retI32(0);
            return retI32(peripheralResult(state, frame, (int) args[3], (int) args[4]));
        });

        addEnv(sink, "periph_take_pending", I32_I32, RET_I32, (inst, args) -> {
            byte[] pending = state.pendingPeripheralResult;
            if (pending == null) return retI32(-1);
            int cap = (int) args[1];
            if (pending.length > cap) return retI32(pending.length);
            state.pendingPeripheralResult = null;
            state.mem().writeBytes((int) args[0], pending, 0, pending.length);
            return retI32(pending.length);
        });

        // === Sleep / time ===

        addEnv(sink, "sleep_ms", I32, RET_NONE, (inst, args) -> {
            if (childBridge != null) {
                childBridge.sleepMs((int) args[0]);
            }
            return null;
        });

        addEnv(sink, "get_time_ms", NO_PARAMS, RET_I64,
                (inst, args) -> retI64(System.currentTimeMillis()));

        // === Raw packet capture host functions (for tcpdump) ===

        addEnv(sink, "net_set_promiscuous_on", I32_I32, RET_I32, (inst, args) -> {
            if (childBridge == null) return retI32(-1);
            return retI32(childBridge.netSetPromiscuousOn((int) args[0], (int) args[1]));
        });

        addEnv(sink, "net_pcap_enable", I32_I32, RET_I32, (inst, args) -> {
            if (childBridge == null) return retI32(-1);
            return retI32(childBridge.netPcapEnable((int) args[0], (int) args[1]));
        });

        addEnv(sink, "net_pcap_rx", I32_I32_I32, RET_I32, (inst, args) -> {
            if (childBridge == null) return retI32(-1);
            int index = (int) args[0];
            int bufPtr = (int) args[1];
            int bufLen = (int) args[2];
            if (bufLen <= 0) return retI32(-1);
            byte[] frame = childBridge.netPcapRx(index);
            if (frame == null) return retI32(-1);
            int writeLen = Math.min(frame.length, bufLen);
            state.mem().writeBytes(bufPtr, frame, 0, writeLen);
            return retI32(writeLen);
        });

        // === Video playback host functions ===

        addEnv(sink, "video_open", I32x5, RET_I32, (inst, args) -> {
            if (childBridge == null) return retI32(-1);
            String path = state.mem().readString((int) args[0], (int) args[1]);
            return retI32(childBridge.videoOpen(path, (int) args[2], (int) args[3], (int) args[4]));
        });

        addEnv(sink, "video_get_info", I32_I32, RET_I32, (inst, args) -> {
            if (childBridge == null) return retI32(-1);
            int handle = (int) args[0];
            int outPtr = (int) args[1];
            var info = childBridge.videoGetInfo(handle);
            if (info == null) return retI32(-1);
            WasmMemory mem = state.mem();
            mem.writeInt(outPtr,      info.width);
            mem.writeInt(outPtr + 4,  info.height);
            mem.writeInt(outPtr + 8,  info.fpsNum);
            mem.writeInt(outPtr + 12, info.fpsDen);
            mem.writeLong(outPtr + 16, info.frameCount);
            mem.writeLong(outPtr + 24, info.durationMs);
            return retI32(0);
        });

        addEnv(sink, "video_decode_to_gfx", I32_I32, RET_I64, (inst, args) -> {
            if (childBridge == null) return retI64(-2L);
            return retI64(childBridge.videoDecodeToGfx((int) args[0], (int) args[1]));
        });

        addEnv(sink, "video_seek",
                List.of(WasmValType.I32, WasmValType.I64), RET_I32, (inst, args) -> {
            if (childBridge == null) return retI32(-1);
            return retI32(childBridge.videoSeek((int) args[0], args[1]));
        });

        addEnv(sink, "video_close", I32, RET_I32, (inst, args) -> {
            if (childBridge == null) return retI32(-1);
            return retI32(childBridge.videoClose((int) args[0]));
        });

        addEnv(sink, "gfx_init", I32_I32_I32, RET_I32, (inst, args) -> {
            if (childBridge == null) return retI32(-1);
            return retI32(childBridge.gfxInit((int) args[0], (int) args[1], (int) args[2]));
        });

        addEnv(sink, "gfx_set_mode", I32_I32, RET_I32, (inst, args) -> {
            if (childBridge == null) return retI32(-1);
            return retI32(childBridge.gfxSetMode((int) args[0], (int) args[1]));
        });

        addEnv(sink, "screen_query_dims", I32, RET_I32, (inst, args) -> {
            if (childBridge == null) return retI32(-1);
            long packed = childBridge.screenQueryDims();
            if (packed < 0) return retI32(-1);
            int outPtr = (int) args[0];
            WasmMemory mem = state.mem();
            mem.writeInt(outPtr, (int) (packed >>> 32));
            mem.writeInt(outPtr + 4, (int) (packed & 0xFFFFFFFFL));
            return retI32(0);
        });

        addEnv(sink, "screen_set_power", I32, RET_NONE, (inst, args) -> {
            if (childBridge == null) return null;
            childBridge.screenSetPower(((int) args[0]) != 0);
            return null;
        });

        addEnv(sink, "screen_set_pixel_format", I32, RET_NONE, (inst, args) -> {
            if (childBridge == null) return null;
            childBridge.screenSetPixelFormat((int) args[0]);
            return null;
        });

        addEnv(sink, "screen_put_frame_rgba", I32_I32_I32, RET_I32, (inst, args) -> {
            if (childBridge == null) return retI32(-1);
            int dataPtr = (int) args[0];
            int w = (int) args[1];
            int h = (int) args[2];
            int byteCount = w * h * 4;
            if (w <= 0 || h <= 0 || byteCount <= 0) return retI32(-1);
            byte[] rgba = state.mem().readBytes(dataPtr, byteCount);
            return retI32(childBridge.screenPutFrameRgba(w, h, rgba));
        });

        addEnv(sink, "gfx_blit_rect", I32x8, RET_I32, (inst, args) -> {
            if (childBridge == null) return retI32(-1);
            int target = (int) args[0];
            int x = (int) args[1];
            int y = (int) args[2];
            int w = (int) args[3];
            int h = (int) args[4];
            int bufPtr = (int) args[5];
            int bufLen = (int) args[6];
            int format = (int) args[7];
            if (!com.example.evanscomputermod.computer.TerminalDisplay.isValidPixelFormat(format)) return retI32(-1);
            int bpp = com.example.evanscomputermod.computer.TerminalDisplay.bytesPerPixel(format);
            if (w <= 0 || h <= 0 || w > 4096 || h > 4096) return retI32(-1);
            int needed = w * h * bpp;
            if (bufLen < needed) return retI32(-1);
            byte[] pixels = state.mem().readBytes(bufPtr, needed);
            return retI32(childBridge.gfxBlitRect(target, x, y, w, h, pixels, format));
        });

        // --- Display device: double buffering, palettes, vblank (see DisplayDevice) ---

        addEnv(sink, "gfx_init2", I32x5, RET_I32, (inst, args) -> {
            if (childBridge == null) return retI32(-1);
            return retI32(childBridge.gfxInit2((int) args[0], (int) args[1], (int) args[2],
                    (int) args[3], (int) args[4]));
        });

        addEnv(sink, "gfx_set_format", I32_I32, RET_I32, (inst, args) -> {
            if (childBridge == null) return retI32(-1);
            return retI32(childBridge.gfxSetFormat((int) args[0], (int) args[1]));
        });

        addEnv(sink, "gfx_set_palette", I32x4, RET_I32, (inst, args) -> {
            if (childBridge == null) return retI32(-1);
            int count = (int) args[2];
            if (count <= 0 || count > 256) return retI32(-1);
            byte[] rgb = state.mem().readBytes((int) args[3], count * 3);
            return retI32(childBridge.gfxSetPalette((int) args[0], (int) args[1], rgb));
        });

        addEnv(sink, "gfx_present", I32_I32, RET_I64, (inst, args) -> {
            if (childBridge == null) return retI64(-1);
            long rc = childBridge.gfxPresent((int) args[0], (int) args[1]);
            throwIfInterrupted(rc);
            return retI64(rc);
        });

        addEnv(sink, "gfx_wait_vblank", I32, RET_I64, (inst, args) -> {
            if (childBridge == null) return retI64(-1);
            long rc = childBridge.gfxWaitVblank((int) args[0]);
            throwIfInterrupted(rc);
            return retI64(rc);
        });

        addEnv(sink, "gfx_set_refresh", I32_I32, RET_I32, (inst, args) -> {
            if (childBridge == null) return retI32(-1);
            return retI32(childBridge.gfxSetRefresh((int) args[0], (int) args[1]));
        });

        // (target, out) -> 0 / -1. out: u32 width, height, format, mode, refresh_hz,
        // flags, owner_pid, _pad; u64 vblank, presented (48 bytes, little-endian).
        addEnv(sink, "gfx_info", I32_I32, RET_I32, (inst, args) -> {
            if (childBridge == null) return retI32(-1);
            var info = childBridge.gfxInfo((int) args[0]);
            if (info == null) return retI32(-1);
            int out = (int) args[1];
            WasmMemory mem = state.mem();
            mem.writeInt(out, info.width());
            mem.writeInt(out + 4, info.height());
            mem.writeInt(out + 8, info.format());
            mem.writeInt(out + 12, info.mode());
            mem.writeInt(out + 16, info.refreshHz());
            mem.writeInt(out + 20, info.flags());
            mem.writeInt(out + 24, info.ownerPid());
            mem.writeInt(out + 28, 0);
            mem.writeLong(out + 32, info.vblank());
            mem.writeLong(out + 40, info.presented());
            return retI32(0);
        });

        addEnv(sink, "mouse_capture_start", NO_PARAMS, RET_I32, (inst, args) -> {
            if (childBridge == null) return retI32(0);
            return retI32(childBridge.mouseCaptureStart());
        });

        addEnv(sink, "mouse_capture_stop", NO_PARAMS, RET_NONE, (inst, args) -> {
            if (childBridge == null) return null;
            childBridge.mouseCaptureStop();
            return null;
        });

        addEnv(sink, "mouse_capture_is_active", NO_PARAMS, RET_I32, (inst, args) -> {
            if (childBridge == null) return retI32(0);
            return retI32(childBridge.mouseCaptureIsActive());
        });

        addEnv(sink, "mouse_poll", I32, RET_I32, (inst, args) -> {
            if (childBridge == null) return retI32(0);
            int bufPtr = (int) args[0];
            byte[] out = new byte[10];
            int rc = childBridge.mousePoll(out);
            if (rc == 1) {
                state.mem().writeBytes(bufPtr, out, 0, 10);
            }
            return retI32(rc);
        });

        addWasi(sink, "poll_oneoff", I32x4, RET_I32, (inst, argsArr) -> {
            int inPtr = (int) argsArr[0];
            int outPtr = (int) argsArr[1];
            int nSubs = (int) argsArr[2];
            int neventsPtr = (int) argsArr[3];

            WasmMemory mem = state.mem();

            long minTimeoutNanos = Long.MAX_VALUE;
            long minUserdata = 0;
            boolean haveClockSub = false;

            for (int i = 0; i < nSubs; i++) {
                int subPtr = inPtr + i * 48;
                long userdata = mem.readLong(subPtr);
                int tag = mem.readByte(subPtr + 8) & 0xFF;
                if (tag == 0) {
                    int clockId = mem.readInt(subPtr + 16);
                    long timeout = mem.readLong(subPtr + 24);
                    int flags = mem.readShort(subPtr + 40) & 0xFFFF;
                    long relNanos;
                    if ((flags & 1) != 0) {
                        long nowNanos = clockNow(clockId);
                        if (nowNanos < 0) nowNanos = clockNow(CLOCK_MONOTONIC);
                        relNanos = Math.max(0, timeout - nowNanos);
                    } else {
                        relNanos = timeout;
                    }
                    if (relNanos < minTimeoutNanos) {
                        minTimeoutNanos = relNanos;
                        minUserdata = userdata;
                    }
                    haveClockSub = true;
                }
            }

            if (haveClockSub && minTimeoutNanos > 0) {
                sleepNanos(Math.min(60_000_000_000L, minTimeoutNanos));
            }

            if (haveClockSub) {
                for (int i = 0; i < 32; i++) mem.writeByte(outPtr + i, (byte) 0);
                mem.writeLong(outPtr, minUserdata);
                mem.writeShort(outPtr + 8, (short) 0);
                mem.writeByte(outPtr + 10, (byte) 0);
                mem.writeInt(neventsPtr, 1);
            } else {
                mem.writeInt(neventsPtr, 0);
            }
            return retI32(ERRNO_SUCCESS);
        });
    }

    /**
     * Register POSIX socket host functions for a child WASI process.
     * Imported under the {@code env} module by ecm-host-abi's socket.rs.
     */
    /**
     * Socket host functions for a child. Each call is proxied to the kernel's
     * non-blocking socket syscalls through {@link NetIpcBridge}; the child
     * thread waits while the kernel answers IPC_PENDING. Child-visible return
     * values are unchanged: accept -2 = timeout, recv 0 = EOF / -2 = timeout,
     * recvfrom 0 = timeout, -1 = error.
     */
    public static void registerSocketFunctions(WasiState state, List<WasmHostFunc> sink,
                                                NetIpcBridge bridge, int sessionId) {
        FdTable fdTable = state.fdTable;
        /** Max payload per send/sendto request (fits the kernel's arg region). */
        final int MAX_SEND = 4096;

        addEnv(sink, "sock_socket", I32_I32_I32, RET_I32, (inst, args) -> {
            byte[] a = new byte[12];
            ByteBuffer ab = ByteBuffer.wrap(a).order(ByteOrder.LITTLE_ENDIAN);
            ab.putInt(0, (int) args[0]);
            ab.putInt(4, (int) args[1]);
            ab.putInt(8, (int) args[2]);
            NetIpcBridge.Result r = bridge.call(sessionId, SocketFd.SOCK_SOCKET, a);
            if (r.status < 0) return retI32(-1);
            return retI32(fdTable.allocate(new SocketFd(r.status, sessionId, bridge)));
        });

        addEnv(sink, "sock_bind", I32_I32_I32, RET_I32, (inst, args) -> {
            if (!(fdTable.get((int) args[0]) instanceof SocketFd sock)) return retI32(-1);
            byte[] addr = state.mem().readBytes((int) args[1], Math.min((int) args[2], 16));
            return retI32(bridge.call(sessionId, SocketFd.SOCK_BIND, sockArgs(sock, addr)).status);
        });

        addEnv(sink, "sock_connect", I32_I32_I32, RET_I32, (inst, args) -> {
            if (!(fdTable.get((int) args[0]) instanceof SocketFd sock)) return retI32(-1);
            byte[] addr = state.mem().readBytes((int) args[1], Math.min((int) args[2], 16));
            return retI32(bridge.call(sessionId, SocketFd.SOCK_CONNECT, sockArgs(sock, addr)).status);
        });

        addEnv(sink, "sock_listen", I32_I32, RET_I32, (inst, args) -> {
            if (!(fdTable.get((int) args[0]) instanceof SocketFd sock)) return retI32(-1);
            byte[] a = new byte[8];
            ByteBuffer ab = ByteBuffer.wrap(a).order(ByteOrder.LITTLE_ENDIAN);
            ab.putInt(0, sock.getKernelSocketId());
            ab.putInt(4, (int) args[1]);
            return retI32(bridge.call(sessionId, SocketFd.SOCK_LISTEN, a).status);
        });

        addEnv(sink, "sock_accept", I32_I32_I32, RET_I32, (inst, args) -> {
            if (!(fdTable.get((int) args[0]) instanceof SocketFd sock)) return retI32(-1);
            int addrPtr = (int) args[1];
            int addrLenPtr = (int) args[2];
            NetIpcBridge.Result r = bridge.call(sessionId, SocketFd.SOCK_ACCEPT, SocketFd.encodeI32(sock.getKernelSocketId()));
            if (r.status < 0) return retI32(r.status);
            int newFd = fdTable.allocate(new SocketFd(r.status, sessionId, bridge));
            if (addrPtr != 0 && r.payload.length >= 16) {
                state.mem().writeBytes(addrPtr, r.payload, 0, 16);
                if (addrLenPtr != 0) state.mem().writeInt(addrLenPtr, 16);
            }
            return retI32(newFd);
        });

        addEnv(sink, "sock_send", I32x4, RET_I32, (inst, args) -> {
            if (!(fdTable.get((int) args[0]) instanceof SocketFd sock)) return retI32(-1);
            int len = Math.max(0, Math.min((int) args[2], MAX_SEND));
            byte[] data = state.mem().readBytes((int) args[1], len);
            return retI32(bridge.call(sessionId, SocketFd.SOCK_SEND, SocketFd.sendArgs(sock.getKernelSocketId(), data)).status);
        });

        addEnv(sink, "sock_recv", I32x4, RET_I32, (inst, args) -> {
            if (!(fdTable.get((int) args[0]) instanceof SocketFd sock)) return retI32(-1);
            int bufPtr = (int) args[1];
            int bufLen = Math.max(0, (int) args[2]);
            NetIpcBridge.Result r = bridge.call(sessionId, SocketFd.SOCK_RECV,
                    SocketFd.recvArgs(sock.getKernelSocketId(), bufLen, (int) args[3]));
            if (r.status <= 0) return retI32(r.status);
            int n = Math.min(Math.min(r.status, r.payload.length), bufLen);
            state.mem().writeBytes(bufPtr, r.payload, 0, n);
            return retI32(n);
        });

        addEnv(sink, "sock_sendto", I32x6, RET_I32, (inst, args) -> {
            if (!(fdTable.get((int) args[0]) instanceof SocketFd sock)) return retI32(-1);
            WasmMemory mem = state.mem();
            int len = Math.max(0, Math.min((int) args[2], MAX_SEND));
            byte[] data = mem.readBytes((int) args[1], len);
            byte[] addr = mem.readBytes((int) args[4], Math.max(0, Math.min((int) args[5], 16)));
            byte[] a = new byte[4 + 2 + addr.length + 2 + data.length];
            ByteBuffer ab = ByteBuffer.wrap(a).order(ByteOrder.LITTLE_ENDIAN);
            ab.putInt(0, sock.getKernelSocketId());
            ab.putShort(4, (short) addr.length);
            System.arraycopy(addr, 0, a, 6, addr.length);
            ab.putShort(6 + addr.length, (short) data.length);
            System.arraycopy(data, 0, a, 8 + addr.length, data.length);
            return retI32(bridge.call(sessionId, SocketFd.SOCK_SENDTO, a).status);
        });

        addEnv(sink, "sock_recvfrom", I32x6, RET_I32, (inst, args) -> {
            if (!(fdTable.get((int) args[0]) instanceof SocketFd sock)) return retI32(-1);
            int bufPtr = (int) args[1];
            int bufLen = Math.max(0, (int) args[2]);
            int addrPtr = (int) args[4];
            int addrLenPtr = (int) args[5];
            NetIpcBridge.Result r = bridge.call(sessionId, SocketFd.SOCK_RECVFROM,
                    SocketFd.recvArgs(sock.getKernelSocketId(), bufLen, (int) args[3]));
            if (r.status <= 0) return retI32(r.status);
            WasmMemory mem = state.mem();
            // payload = [sockaddr_in 16][data]
            if (addrPtr != 0 && r.payload.length >= 16) {
                mem.writeBytes(addrPtr, r.payload, 0, 16);
                if (addrLenPtr != 0) mem.writeInt(addrLenPtr, 16);
            }
            int n = Math.min(Math.min(r.status, r.payload.length - 16), bufLen);
            if (n > 0) mem.writeBytes(bufPtr, r.payload, 16, n);
            return retI32(Math.max(n, 0));
        });

        addEnv(sink, "sock_setsockopt", I32x5, RET_I32, (inst, args) -> {
            if (!(fdTable.get((int) args[0]) instanceof SocketFd sock)) return retI32(-1);
            byte[] optval = state.mem().readBytes((int) args[3], Math.max(0, Math.min((int) args[4], 64)));
            byte[] a = new byte[12 + optval.length];
            ByteBuffer ab = ByteBuffer.wrap(a).order(ByteOrder.LITTLE_ENDIAN);
            ab.putInt(0, sock.getKernelSocketId());
            ab.putInt(4, (int) args[1]);
            ab.putInt(8, (int) args[2]);
            System.arraycopy(optval, 0, a, 12, optval.length);
            return retI32(bridge.call(sessionId, SocketFd.SOCK_SETSOCKOPT, a).status);
        });

        addEnv(sink, "sock_getsockname", I32_I32_I32, RET_I32, (inst, args) ->
                retI32(writeName(state, bridge.call(sessionId, SocketFd.SOCK_GETSOCKNAME,
                        sockIdArgs(fdTable, (int) args[0])), (int) args[1], (int) args[2])));

        addEnv(sink, "sock_getpeername", I32_I32_I32, RET_I32, (inst, args) ->
                retI32(writeName(state, bridge.call(sessionId, SocketFd.SOCK_GETPEERNAME,
                        sockIdArgs(fdTable, (int) args[0])), (int) args[1], (int) args[2])));

        addEnv(sink, "sock_shutdown", I32_I32, RET_I32, (inst, args) -> {
            if (!(fdTable.get((int) args[0]) instanceof SocketFd sock)) return retI32(-1);
            byte[] a = new byte[8];
            ByteBuffer ab = ByteBuffer.wrap(a).order(ByteOrder.LITTLE_ENDIAN);
            ab.putInt(0, sock.getKernelSocketId());
            ab.putInt(4, (int) args[1]);
            return retI32(bridge.call(sessionId, SocketFd.SOCK_SHUTDOWN, a).status);
        });

        // pollfd = [fd i32][events i16][revents i16]; non-socket fds report POLLNVAL.
        addEnv(sink, "sock_poll", I32_I32_I32, RET_I32, (inst, args) -> {
            int fdsPtr = (int) args[0];
            int n = (int) args[1];
            if (n < 0 || n > 64) return retI32(-1);
            WasmMemory mem = state.mem();
            byte[] a = new byte[8 + n * 8];
            ByteBuffer ab = ByteBuffer.wrap(a).order(ByteOrder.LITTLE_ENDIAN);
            ab.putInt(0, (int) args[2]);
            ab.putInt(4, n);
            boolean[] invalid = new boolean[n];
            for (int i = 0; i < n; i++) {
                int fd = mem.readInt(fdsPtr + i * 8);
                short events = mem.readShort(fdsPtr + i * 8 + 4);
                int id = fdTable.get(fd) instanceof SocketFd sock ? sock.getKernelSocketId() : -1;
                invalid[i] = id < 0;
                ab.putInt(8 + i * 8, id);
                ab.putShort(12 + i * 8, events);
            }
            NetIpcBridge.Result r = bridge.call(sessionId, SocketFd.SOCK_POLL, a);
            if (r.status < 0) return retI32(-1);
            ByteBuffer rb = ByteBuffer.wrap(r.payload).order(ByteOrder.LITTLE_ENDIAN);
            int ready = 0;
            for (int i = 0; i < n; i++) {
                short rev = invalid[i] ? (short) 0x20
                        : (r.payload.length >= 2 * i + 2 ? rb.getShort(2 * i) : 0);
                if (rev != 0) ready++;
                mem.writeShort(fdsPtr + i * 8 + 6, rev);
            }
            return retI32(ready);
        });

        addEnv(sink, "sock_getaddrinfo", I32x4, RET_I32, (inst, args) -> {
            String host = state.mem().readString((int) args[0], (int) args[1]);
            byte[] hostBytes = host.getBytes(StandardCharsets.UTF_8);
            if (hostBytes.length > 255) return retI32(-1);
            byte[] a = new byte[2 + hostBytes.length];
            ByteBuffer.wrap(a).order(ByteOrder.LITTLE_ENDIAN).putShort(0, (short) hostBytes.length);
            System.arraycopy(hostBytes, 0, a, 2, hostBytes.length);
            NetIpcBridge.Result r = bridge.call(sessionId, SocketFd.SOCK_GETADDRINFO, a);
            if (r.status < 0 || r.payload.length < 16) return retI32(-1);
            int n = Math.min(16, (int) args[3]);
            state.mem().writeBytes((int) args[2], r.payload, 0, n);
            return retI32(n);
        });

        addEnv(sink, "get_time_ms", NO_PARAMS, RET_I64,
                (inst, args) -> retI64(System.currentTimeMillis()));

        registerShellSessionFunctions(state, sink, bridge, sessionId);
        registerWifiFunctions(state, sink, bridge, sessionId);
    }

    /**
     * {@code wifi_ctl(req, len, reply, cap, status_out) -> reply length (truncated
     * to cap), -1 on failure}: one text request to the kernel's wlan0 control
     * channel (kernel net/wifi.rs, ecm-host-abi wifi.rs). Used by iw,
     * wpa_supplicant, wpa_cli and tcpdump -i wlan0.
     */
    private static void registerWifiFunctions(WasiState state, List<WasmHostFunc> sink,
                                              NetIpcBridge bridge, int pid) {
        addEnv(sink, "wifi_ctl", I32x5, RET_I32, (inst, args) -> {
            int len = Math.max(0, Math.min((int) args[1], 6000));
            byte[] req = state.mem().readBytes((int) args[0], len);
            NetIpcBridge.Result r = bridge.call(pid, SocketFd.WIFI_CTL, req);
            int n = Math.min(r.payload.length, Math.max(0, (int) args[3]));
            if (n > 0) state.mem().writeBytes((int) args[2], r.payload, 0, n);
            state.mem().writeInt((int) args[4], r.status);
            return retI32(n);
        });
    }

    /**
     * Remote shell sessions for sshd (ecm-host-abi ipc.rs). The kernel hosts
     * the shell; these calls relay a client's keystrokes and the shell's
     * output. Return values: spawn -> session id or -1; write -> bytes;
     * read -> bytes (0 none, -1 error/closed); read_blocking -> bytes, 0 on
     * timeout, -1 closed; status -> 0 running, 1 exited, -1 invalid.
     */
    private static void registerShellSessionFunctions(WasiState state, List<WasmHostFunc> sink,
                                                      NetIpcBridge bridge, int pid) {
        addEnv(sink, "ipc_spawn_shell", I32_I32, RET_I32, (inst, args) -> {
            byte[] user = state.mem().readBytes((int) args[0], Math.max(0, Math.min((int) args[1], 64)));
            byte[] a = new byte[2 + user.length];
            ByteBuffer.wrap(a).order(ByteOrder.LITTLE_ENDIAN).putShort(0, (short) user.length);
            System.arraycopy(user, 0, a, 2, user.length);
            return retI32(bridge.call(pid, SocketFd.SESSION_SPAWN, a).status);
        });

        addEnv(sink, "ipc_session_write", I32_I32_I32, RET_I32, (inst, args) -> {
            int id = (int) args[0];
            byte[] data = state.mem().readBytes((int) args[1], Math.max(0, (int) args[2]));
            int written = 0;
            while (written < data.length) {
                int n = Math.min(4096, data.length - written);
                byte[] a = new byte[4 + 2 + n];
                ByteBuffer ab = ByteBuffer.wrap(a).order(ByteOrder.LITTLE_ENDIAN);
                ab.putInt(0, id);
                ab.putShort(4, (short) n);
                System.arraycopy(data, written, a, 6, n);
                int st = bridge.call(pid, SocketFd.SESSION_WRITE, a).status;
                if (st < 0) return retI32(written > 0 ? written : -1);
                written += n;
            }
            return retI32(written);
        });

        addEnv(sink, "ipc_session_read", I32_I32_I32, RET_I32, (inst, args) ->
                retI32(sessionRead(state, bridge, pid, SocketFd.SESSION_READ,
                        (int) args[0], (int) args[1], (int) args[2], 0)));

        addEnv(sink, "ipc_session_read_blocking", I32x4, RET_I32, (inst, args) ->
                retI32(sessionRead(state, bridge, pid, SocketFd.SESSION_READ_BLOCKING,
                        (int) args[0], (int) args[1], (int) args[2], (int) args[3])));

        addEnv(sink, "ipc_session_status", I32, RET_I32, (inst, args) ->
                retI32(bridge.call(pid, SocketFd.SESSION_STATUS, SocketFd.encodeI32((int) args[0])).status));

        addEnv(sink, "ipc_session_close", I32, RET_I32, (inst, args) ->
                retI32(bridge.call(pid, SocketFd.SESSION_CLOSE, SocketFd.encodeI32((int) args[0])).status));

        addEnv(sink, "ipc_session_resize", I32_I32_I32, RET_I32, (inst, args) -> {
            byte[] a = new byte[12];
            ByteBuffer ab = ByteBuffer.wrap(a).order(ByteOrder.LITTLE_ENDIAN);
            ab.putInt(0, (int) args[0]);
            ab.putInt(4, (int) args[1]);
            ab.putInt(8, (int) args[2]);
            return retI32(bridge.call(pid, SocketFd.SESSION_RESIZE, a).status);
        });
    }

    private static int sessionRead(WasiState state, NetIpcBridge bridge, int pid, int syscall,
                                   int id, int bufPtr, int bufLen, int timeoutMs) {
        byte[] a = new byte[12];
        ByteBuffer ab = ByteBuffer.wrap(a).order(ByteOrder.LITTLE_ENDIAN);
        ab.putInt(0, id);
        ab.putInt(4, Math.max(0, Math.min(bufLen, 4096)));
        ab.putInt(8, Math.max(0, timeoutMs));
        NetIpcBridge.Result r = bridge.call(pid, syscall, a);
        if (r.status <= 0) return r.status;
        int n = Math.min(Math.min(r.status, r.payload.length), bufLen);
        state.mem().writeBytes(bufPtr, r.payload, 0, n);
        return n;
    }

    private static byte[] sockArgs(SocketFd sock, byte[] tail) {
        byte[] a = new byte[4 + tail.length];
        ByteBuffer.wrap(a).order(ByteOrder.LITTLE_ENDIAN).putInt(0, sock.getKernelSocketId());
        System.arraycopy(tail, 0, a, 4, tail.length);
        return a;
    }

    private static byte[] sockIdArgs(FdTable fdTable, int fd) {
        return fdTable.get(fd) instanceof SocketFd sock
                ? SocketFd.encodeI32(sock.getKernelSocketId())
                : SocketFd.encodeI32(-1);
    }

    /** getsockname/getpeername: status 0 + 16-byte sockaddr payload. */
    private static int writeName(WasiState state, NetIpcBridge.Result r, int addrPtr, int addrLenPtr) {
        if (r.status < 0 || r.payload.length < 16) return -1;
        state.mem().writeBytes(addrPtr, r.payload, 0, 16);
        if (addrLenPtr != 0) state.mem().writeInt(addrLenPtr, 16);
        return 0;
    }

    // --- registration helpers ---

    private static void subscribePeripheralEvents(WasiState state, ChildHostBridge childBridge) {
        if (state.peripheralEvents == null) {
            state.peripheralEvents = childBridge.peripheralSubscribe();
        }
    }

    private static int peripheralResult(WasiState state, byte[] frame, int buf, int cap) {
        if (frame.length > cap) {
            state.pendingPeripheralResult = frame;
            return frame.length;
        }
        state.pendingPeripheralResult = null;
        state.mem().writeBytes(buf, frame, 0, frame.length);
        return frame.length;
    }

    private static void addWasi(List<WasmHostFunc> sink, String name,
                                 List<WasmValType> params, List<WasmValType> results,
                                 WasmHostFunc.Handler handler) {
        sink.add(new WasmHostFunc(WASI_NS, name, params, results, handler));
        // Some guest modules (esp. pre-snapshot ones, and the ecm-host-abi
        // tools) declare WASI imports without the namespace. Register both.
        sink.add(new WasmHostFunc("", name, params, results, handler));
    }

    static final int CLOCK_REALTIME = 0;
    static final int CLOCK_MONOTONIC = 1;
    static final int CLOCK_PROCESS_CPUTIME = 2;
    static final int CLOCK_THREAD_CPUTIME = 3;
    /** Monotonic clock origin, so values start near 0 and stay positive. */
    private static final long MONOTONIC_ORIGIN = System.nanoTime();

    /**
     * WASI clocks in nanoseconds: realtime from the wall clock, the others from
     * {@link System#nanoTime()} (monotonic, sub-microsecond). -1 for an unknown id.
     */
    static long clockNow(int id) {
        return switch (id) {
            case CLOCK_REALTIME -> {
                java.time.Instant now = java.time.Instant.now();
                yield now.getEpochSecond() * 1_000_000_000L + now.getNano();
            }
            case CLOCK_MONOTONIC, CLOCK_PROCESS_CPUTIME, CLOCK_THREAD_CPUTIME ->
                    System.nanoTime() - MONOTONIC_ORIGIN;
            default -> -1;
        };
    }

    /**
     * Sleep with sub-millisecond precision (std::thread::sleep lands here via
     * poll_oneoff). A kill interrupts the thread: unwind like the other
     * blocking host calls.
     */
    private static void sleepNanos(long nanos) {
        long deadline = System.nanoTime() + nanos;
        while (true) {
            long remaining = deadline - System.nanoTime();
            if (remaining <= 0) return;
            java.util.concurrent.locks.LockSupport.parkNanos(remaining);
            if (Thread.currentThread().isInterrupted()) {
                throw new RuntimeException("WASI child poll_oneoff interrupted");
            }
        }
    }

    /**
     * A blocking display call that came back {@code E_INTERRUPTED} means the
     * program was killed while waiting: unwind it like any other interrupted
     * host call (ProcessManager maps this to exit code 130).
     */
    private static void throwIfInterrupted(long rc) {
        if (rc == com.example.evanscomputermod.computer.display.DisplayDevice.E_INTERRUPTED) {
            throw new RuntimeException("display wait interrupted");
        }
    }

    private static void addEnv(List<WasmHostFunc> sink, String name,
                                List<WasmValType> params, List<WasmValType> results,
                                WasmHostFunc.Handler handler) {
        sink.add(new WasmHostFunc(ENV_NS, name, params, results, handler));
        sink.add(new WasmHostFunc("", name, params, results, handler));
    }

    // --- low-level helpers ---

    private static Path resolveChildPath(WasiState state, String pathStr) {
        if (pathStr == null) return null;
        if (pathStr.contains("..") || pathStr.startsWith("/")) return null;
        Path filePath = state.storagePath.resolve(pathStr).normalize();
        if (!filePath.startsWith(state.storagePath)) return null;
        return filePath;
    }

    private static int wasifdWrite(WasiState state, int fd, int iovsPtr, int iovsLen, int nwrittenPtr) {
        WasiFileDescriptor desc = state.fdTable.get(fd);
        if (desc == null) {
            EvansComputerMod.LOGGER.debug("WASI fd_write: fd={} NOT FOUND", fd);
            return ERRNO_BADF;
        }

        WasmMemory mem = state.mem();
        int total = 0;

        for (int i = 0; i < iovsLen; i++) {
            int iovAddr = iovsPtr + i * 8;
            int bufPtr = mem.readInt(iovAddr);
            int bufLen = mem.readInt(iovAddr + 4);
            byte[] data = mem.readBytes(bufPtr, bufLen);
            try {
                int written = desc.write(data, 0, bufLen);
                if (written < 0) {
                    EvansComputerMod.LOGGER.debug("WASI fd_write: fd={} write returned {}", fd, written);
                    return ERRNO_BADF;
                }
                total += written;
                if (written < bufLen) {
                    // A non-blocking device took part (or none) of it.
                    if (total == 0 && desc instanceof DeviceFd) return ERRNO_AGAIN;
                    break;
                }
            } catch (DeviceFd.ErrnoException e) {
                if (total > 0) break;
                return e.errno();
            } catch (IOException e) {
                EvansComputerMod.LOGGER.error("WASI fd_write: fd={} IOException", fd, e);
                return ERRNO_BADF;
            }
        }

        if (fd > 2) {
            EvansComputerMod.LOGGER.debug("WASI fd_write: fd={} wrote {} bytes (desc={})", fd, total, desc.getClass().getSimpleName());
        }

        mem.writeInt(nwrittenPtr, total);
        return ERRNO_SUCCESS;
    }

    private static int wasifdRead(WasiState state, int fd, int iovsPtr, int iovsLen, int nreadPtr) {
        WasiFileDescriptor desc = state.fdTable.get(fd);
        if (desc == null) return ERRNO_BADF;

        WasmMemory mem = state.mem();
        int total = 0;

        for (int i = 0; i < iovsLen; i++) {
            int iovAddr = iovsPtr + i * 8;
            int bufPtr = mem.readInt(iovAddr);
            int bufLen = mem.readInt(iovAddr + 4);
            byte[] data = new byte[bufLen];
            try {
                int nread = desc.read(data, 0, bufLen);
                if (nread == PipeFd.WOULD_BLOCK) {
                    if (total > 0) break;
                    return ERRNO_AGAIN;
                }
                if (nread <= 0) break;
                mem.writeBytes(bufPtr, data, 0, nread);
                total += nread;
                if (nread < bufLen) break;
            } catch (DeviceFd.ErrnoException e) {
                if (total > 0) break;
                return e.errno();
            } catch (IOException e) {
                return ERRNO_BADF;
            }
        }

        mem.writeInt(nreadPtr, total);
        return ERRNO_SUCCESS;
    }
}
