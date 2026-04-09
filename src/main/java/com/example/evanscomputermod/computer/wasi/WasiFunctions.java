package com.example.evanscomputermod.computer.wasi;

import com.example.evanscomputermod.EvansComputerMod;
import io.github.kawamuray.wasmtime.*;
import io.github.kawamuray.wasmtime.Val.Type;

import java.io.IOException;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;

/**
 * Provides WASI snapshot_preview1 host functions for child WASM processes.
 * Each child process gets its own Store with its own FdTable.
 *
 * <p>The child's state is stored in the Store's data as a {@link WasiState}.</p>
 */
public class WasiFunctions {

    private static final String WASI_NS = "wasi_snapshot_preview1";
    private static final int ERRNO_SUCCESS = 0;
    private static final int ERRNO_BADF = 8;
    private static final int ERRNO_INVAL = 28;
    private static final int ERRNO_NOSYS = 52;
    private static final int ERRNO_NOENT = 44;

    /**
     * State held in each child process's Store.
     */
    public static class WasiState {
        public final FdTable fdTable;
        public final String[] argv;
        public final Path storagePath;
        public final java.util.Map<String, String> envVars;
        public Memory memory;

        public WasiState(FdTable fdTable, String[] argv, Path storagePath, java.util.Map<String, String> envVars) {
            this.fdTable = fdTable;
            this.argv = argv;
            this.storagePath = storagePath;
            this.envVars = envVars != null ? envVars : java.util.Map.of();
        }
    }

    /**
     * Register all WASI host functions on the given linker.
     */
    public static void register(Store<WasiState> store, List<Func> funcs,
                                 java.util.Map<String, Extern> funcMap) {
        // fd_write(fd, iovs_ptr, iovs_len, nwritten_ptr) -> errno
        addFunc(store, funcs, funcMap, "fd_write",
                new Type[]{Type.I32, Type.I32, Type.I32, Type.I32}, new Type[]{Type.I32},
                (caller, params, results) -> {
                    int fd = params[0].i32();
                    int iovsPtr = params[1].i32();
                    int iovsLen = params[2].i32();
                    int nwrittenPtr = params[3].i32();
                    results[0] = Val.fromI32(wasifdWrite(store, fd, iovsPtr, iovsLen, nwrittenPtr));
                });

        // fd_read(fd, iovs_ptr, iovs_len, nread_ptr) -> errno
        addFunc(store, funcs, funcMap, "fd_read",
                new Type[]{Type.I32, Type.I32, Type.I32, Type.I32}, new Type[]{Type.I32},
                (caller, params, results) -> {
                    int fd = params[0].i32();
                    int iovsPtr = params[1].i32();
                    int iovsLen = params[2].i32();
                    int nreadPtr = params[3].i32();
                    results[0] = Val.fromI32(wasifdRead(store, fd, iovsPtr, iovsLen, nreadPtr));
                });

        // fd_close(fd) -> errno
        addFunc(store, funcs, funcMap, "fd_close",
                new Type[]{Type.I32}, new Type[]{Type.I32},
                (caller, params, results) -> {
                    store.data().fdTable.close(params[0].i32());
                    results[0] = Val.fromI32(ERRNO_SUCCESS);
                });

        // fd_seek(fd, offset, whence, newoffset_ptr) -> errno
        addFunc(store, funcs, funcMap, "fd_seek",
                new Type[]{Type.I32, Type.I64, Type.I32, Type.I32}, new Type[]{Type.I32},
                (caller, params, results) -> {
                    int fd = params[0].i32();
                    WasiFileDescriptor desc = store.data().fdTable.get(fd);
                    if (desc instanceof VfsFileFd vfs) {
                        try {
                            long newOff = vfs.seek(params[1].i64(), params[2].i32());
                            ByteBuffer mem = store.data().memory.buffer(store);
                            mem.putLong(params[3].i32(), newOff);
                            results[0] = Val.fromI32(ERRNO_SUCCESS);
                        } catch (IOException e) {
                            results[0] = Val.fromI32(ERRNO_INVAL);
                        }
                    } else {
                        results[0] = Val.fromI32(ERRNO_NOSYS);
                    }
                });

        // fd_fdstat_get(fd, buf_ptr) -> errno
        addFunc(store, funcs, funcMap, "fd_fdstat_get",
                new Type[]{Type.I32, Type.I32}, new Type[]{Type.I32},
                (caller, params, results) -> {
                    int fd = params[0].i32();
                    int bufPtr = params[1].i32();
                    ByteBuffer mem = store.data().memory.buffer(store);
                    // fdstat: filetype(1), fdflags(2), rights_base(8), rights_inheriting(8) = 24 bytes
                    for (int i = 0; i < 24; i++) mem.put(bufPtr + i, (byte) 0);
                    if (fd <= 2) {
                        mem.put(bufPtr, (byte) 2); // FILETYPE_CHARACTER_DEVICE
                    } else if (fd == 3) {
                        mem.put(bufPtr, (byte) 3); // FILETYPE_DIRECTORY
                    } else {
                        mem.put(bufPtr, (byte) 4); // FILETYPE_REGULAR_FILE
                    }
                    // Set all rights
                    mem.putLong(bufPtr + 8, 0xFFFFFFFFL);
                    mem.putLong(bufPtr + 16, 0xFFFFFFFFL);
                    results[0] = Val.fromI32(ERRNO_SUCCESS);
                });

        // fd_fdstat_set_flags(fd, flags) -> errno
        addFunc(store, funcs, funcMap, "fd_fdstat_set_flags",
                new Type[]{Type.I32, Type.I32}, new Type[]{Type.I32},
                (caller, params, results) -> results[0] = Val.fromI32(ERRNO_SUCCESS));

        // fd_filestat_get(fd, buf_ptr) -> errno
        addFunc(store, funcs, funcMap, "fd_filestat_get",
                new Type[]{Type.I32, Type.I32}, new Type[]{Type.I32},
                (caller, params, results) -> {
                    int fd = params[0].i32();
                    int bufPtr = params[1].i32();
                    ByteBuffer mem = store.data().memory.buffer(store);
                    // filestat: dev(8) ino(8) filetype(1)+pad(7) nlink(8) size(8) atim(8) mtim(8) ctim(8) = 64
                    for (int i = 0; i < 64; i++) mem.put(bufPtr + i, (byte) 0);

                    WasiFileDescriptor desc = store.data().fdTable.get(fd);
                    if (desc instanceof VfsFileFd vfs) {
                        mem.put(bufPtr + 16, (byte) 4); // FILETYPE_REGULAR_FILE
                        try { mem.putLong(bufPtr + 32, vfs.size()); } catch (IOException ignored) {}
                    } else if (desc instanceof DirFd) {
                        mem.put(bufPtr + 16, (byte) 3); // FILETYPE_DIRECTORY
                    } else {
                        mem.put(bufPtr + 16, (byte) 2); // FILETYPE_CHARACTER_DEVICE
                    }
                    results[0] = Val.fromI32(ERRNO_SUCCESS);
                });

        // fd_prestat_get(fd, buf_ptr) -> errno
        addFunc(store, funcs, funcMap, "fd_prestat_get",
                new Type[]{Type.I32, Type.I32}, new Type[]{Type.I32},
                (caller, params, results) -> {
                    int fd = params[0].i32();
                    if (fd == 3) {
                        // Preopened directory "/"
                        ByteBuffer mem = store.data().memory.buffer(store);
                        mem.put(params[1].i32(), (byte) 0); // type = dir
                        mem.order(ByteOrder.LITTLE_ENDIAN);
                        mem.putInt(params[1].i32() + 4, 1); // name length = 1 ("/" or ".")
                        results[0] = Val.fromI32(ERRNO_SUCCESS);
                    } else {
                        results[0] = Val.fromI32(ERRNO_BADF);
                    }
                });

        // fd_prestat_dir_name(fd, path_ptr, path_len) -> errno
        addFunc(store, funcs, funcMap, "fd_prestat_dir_name",
                new Type[]{Type.I32, Type.I32, Type.I32}, new Type[]{Type.I32},
                (caller, params, results) -> {
                    if (params[0].i32() == 3) {
                        ByteBuffer mem = store.data().memory.buffer(store);
                        mem.put(params[1].i32(), (byte) '/');
                        results[0] = Val.fromI32(ERRNO_SUCCESS);
                    } else {
                        results[0] = Val.fromI32(ERRNO_BADF);
                    }
                });

        // proc_exit(code) -> noreturn
        addFunc(store, funcs, funcMap, "proc_exit",
                new Type[]{Type.I32}, new Type[]{},
                (caller, params, results) -> {
                    throw new WasiExitException(params[0].i32());
                });

        // args_sizes_get(argc_ptr, argv_buf_size_ptr) -> errno
        addFunc(store, funcs, funcMap, "args_sizes_get",
                new Type[]{Type.I32, Type.I32}, new Type[]{Type.I32},
                (caller, params, results) -> {
                    ByteBuffer mem = store.data().memory.buffer(store);
                    mem.order(ByteOrder.LITTLE_ENDIAN);
                    String[] argv = store.data().argv;
                    int totalSize = 0;
                    for (String arg : argv) totalSize += arg.getBytes(StandardCharsets.UTF_8).length + 1;
                    mem.putInt(params[0].i32(), argv.length);
                    mem.putInt(params[1].i32(), totalSize);
                    results[0] = Val.fromI32(ERRNO_SUCCESS);
                });

        // args_get(argv_ptr, argv_buf_ptr) -> errno
        addFunc(store, funcs, funcMap, "args_get",
                new Type[]{Type.I32, Type.I32}, new Type[]{Type.I32},
                (caller, params, results) -> {
                    ByteBuffer mem = store.data().memory.buffer(store);
                    mem.order(ByteOrder.LITTLE_ENDIAN);
                    String[] argv = store.data().argv;
                    int argvPtr = params[0].i32();
                    int bufPtr = params[1].i32();
                    for (int i = 0; i < argv.length; i++) {
                        mem.putInt(argvPtr + i * 4, bufPtr);
                        byte[] bytes = argv[i].getBytes(StandardCharsets.UTF_8);
                        for (byte b : bytes) mem.put(bufPtr++, b);
                        mem.put(bufPtr++, (byte) 0); // null terminator
                    }
                    results[0] = Val.fromI32(ERRNO_SUCCESS);
                });

        // environ_sizes_get(count_ptr, buf_size_ptr) -> errno
        addFunc(store, funcs, funcMap, "environ_sizes_get",
                new Type[]{Type.I32, Type.I32}, new Type[]{Type.I32},
                (caller, params, results) -> {
                    ByteBuffer mem = store.data().memory.buffer(store);
                    mem.order(ByteOrder.LITTLE_ENDIAN);
                    var env = store.data().envVars;
                    int count = env.size();
                    int bufSize = 0;
                    for (var e : env.entrySet()) {
                        bufSize += e.getKey().length() + 1 + e.getValue().length() + 1; // KEY=VALUE\0
                    }
                    mem.putInt(params[0].i32(), count);
                    mem.putInt(params[1].i32(), bufSize);
                    results[0] = Val.fromI32(ERRNO_SUCCESS);
                });

        // environ_get(environ_ptr, environ_buf_ptr) -> errno
        // Writes an array of pointers at environ_ptr, and serialized KEY=VALUE\0 strings at environ_buf_ptr.
        addFunc(store, funcs, funcMap, "environ_get",
                new Type[]{Type.I32, Type.I32}, new Type[]{Type.I32},
                (caller, params, results) -> {
                    ByteBuffer mem = store.data().memory.buffer(store);
                    mem.order(ByteOrder.LITTLE_ENDIAN);
                    int environPtr = params[0].i32();
                    int bufPtr = params[1].i32();
                    int offset = 0;
                    int idx = 0;
                    for (var e : store.data().envVars.entrySet()) {
                        // Write pointer to this env string
                        mem.putInt(environPtr + idx * 4, bufPtr + offset);
                        // Write KEY=VALUE\0
                        byte[] entry = (e.getKey() + "=" + e.getValue()).getBytes(java.nio.charset.StandardCharsets.UTF_8);
                        for (byte b : entry) {
                            mem.put(bufPtr + offset, b);
                            offset++;
                        }
                        mem.put(bufPtr + offset, (byte) 0); // null terminator
                        offset++;
                        idx++;
                    }
                    results[0] = Val.fromI32(ERRNO_SUCCESS);
                });

        // clock_time_get(clock_id, precision, time_ptr) -> errno
        addFunc(store, funcs, funcMap, "clock_time_get",
                new Type[]{Type.I32, Type.I64, Type.I32}, new Type[]{Type.I32},
                (caller, params, results) -> {
                    ByteBuffer mem = store.data().memory.buffer(store);
                    long nanos = System.currentTimeMillis() * 1_000_000L;
                    mem.putLong(params[2].i32(), nanos);
                    results[0] = Val.fromI32(ERRNO_SUCCESS);
                });

        // random_get(buf_ptr, buf_len) -> errno
        addFunc(store, funcs, funcMap, "random_get",
                new Type[]{Type.I32, Type.I32}, new Type[]{Type.I32},
                (caller, params, results) -> {
                    ByteBuffer mem = store.data().memory.buffer(store);
                    int ptr = params[0].i32();
                    int len = params[1].i32();
                    java.util.Random rng = new java.util.Random();
                    byte[] bytes = new byte[len];
                    rng.nextBytes(bytes);
                    for (int i = 0; i < len; i++) mem.put(ptr + i, bytes[i]);
                    results[0] = Val.fromI32(ERRNO_SUCCESS);
                });

        // sched_yield() -> errno
        addFunc(store, funcs, funcMap, "sched_yield",
                new Type[]{}, new Type[]{Type.I32},
                (caller, params, results) -> {
                    Thread.yield();
                    results[0] = Val.fromI32(ERRNO_SUCCESS);
                });

        // path_open(dirfd, dirflags, path_ptr, path_len, oflags, rights_base, rights_inheriting, fdflags, fd_ptr) -> errno
        addFunc(store, funcs, funcMap, "path_open",
                new Type[]{Type.I32, Type.I32, Type.I32, Type.I32, Type.I32, Type.I64, Type.I64, Type.I32, Type.I32},
                new Type[]{Type.I32},
                (caller, params, results) -> {
                    int pathPtr = params[2].i32();
                    int pathLen = params[3].i32();
                    int oflags = params[4].i32();
                    int fdflags = params[7].i32();
                    int fdOutPtr = params[8].i32();

                    ByteBuffer mem = store.data().memory.buffer(store);
                    byte[] pathBytes = new byte[pathLen];
                    for (int i = 0; i < pathLen; i++) pathBytes[i] = mem.get(pathPtr + i);
                    String pathStr = new String(pathBytes, StandardCharsets.UTF_8);

                    // Sanitize path
                    if (pathStr.contains("..") || pathStr.startsWith("/")) {
                        results[0] = Val.fromI32(ERRNO_NOENT);
                        return;
                    }

                    Path filePath = store.data().storagePath.resolve(pathStr).normalize();
                    if (!filePath.startsWith(store.data().storagePath)) {
                        results[0] = Val.fromI32(ERRNO_NOENT);
                        return;
                    }

                    boolean create = (oflags & 1) != 0; // OFLAGS_CREAT
                    boolean trunc = (oflags & 8) != 0;   // OFLAGS_TRUNC
                    boolean append = (fdflags & 1) != 0;  // FDFLAGS_APPEND

                    if (!Files.exists(filePath) && !create) {
                        results[0] = Val.fromI32(ERRNO_NOENT);
                        return;
                    }

                    try {
                        mem.order(ByteOrder.LITTLE_ENDIAN);
                        if (Files.isDirectory(filePath)) {
                            // Directory: return a DirFd
                            DirFd dirFd = new DirFd(filePath);
                            int newFd = store.data().fdTable.allocate(dirFd);
                            mem.putInt(fdOutPtr, newFd);
                        } else {
                            // Regular file
                            if (create && !Files.exists(filePath)) {
                                Files.createDirectories(filePath.getParent());
                                Files.createFile(filePath);
                            }
                            VfsFileFd vfs = new VfsFileFd(filePath, true, true, append);
                            if (trunc) vfs.seek(0, 0);
                            int newFd = store.data().fdTable.allocate(vfs);
                            mem.putInt(fdOutPtr, newFd);
                        }
                        results[0] = Val.fromI32(ERRNO_SUCCESS);
                    } catch (IOException e) {
                        results[0] = Val.fromI32(ERRNO_NOENT);
                    }
                });

        // path_create_directory(dirfd, path_ptr, path_len) -> errno
        addFunc(store, funcs, funcMap, "path_create_directory",
                new Type[]{Type.I32, Type.I32, Type.I32}, new Type[]{Type.I32},
                (caller, params, results) -> {
                    String pathStr = readString(store, params[1].i32(), params[2].i32());
                    try {
                        Path p = store.data().storagePath.resolve(pathStr).normalize();
                        Files.createDirectories(p);
                        results[0] = Val.fromI32(ERRNO_SUCCESS);
                    } catch (IOException e) {
                        results[0] = Val.fromI32(ERRNO_NOENT);
                    }
                });

        // path_remove_directory(dirfd, path_ptr, path_len) -> errno
        addFunc(store, funcs, funcMap, "path_remove_directory",
                new Type[]{Type.I32, Type.I32, Type.I32}, new Type[]{Type.I32},
                (caller, params, results) -> results[0] = Val.fromI32(ERRNO_NOSYS));

        // path_unlink_file(dirfd, path_ptr, path_len) -> errno
        addFunc(store, funcs, funcMap, "path_unlink_file",
                new Type[]{Type.I32, Type.I32, Type.I32}, new Type[]{Type.I32},
                (caller, params, results) -> {
                    String pathStr = readString(store, params[1].i32(), params[2].i32());
                    try {
                        Path p = store.data().storagePath.resolve(pathStr).normalize();
                        Files.deleteIfExists(p);
                        results[0] = Val.fromI32(ERRNO_SUCCESS);
                    } catch (IOException e) {
                        results[0] = Val.fromI32(ERRNO_NOENT);
                    }
                });

        // path_filestat_get(dirfd, flags, path_ptr, path_len, buf_ptr) -> errno
        addFunc(store, funcs, funcMap, "path_filestat_get",
                new Type[]{Type.I32, Type.I32, Type.I32, Type.I32, Type.I32}, new Type[]{Type.I32},
                (caller, params, results) -> {
                    String pathStr = readString(store, params[2].i32(), params[3].i32());
                    Path p = store.data().storagePath.resolve(pathStr).normalize();
                    if (!Files.exists(p)) {
                        results[0] = Val.fromI32(ERRNO_NOENT);
                        return;
                    }
                    ByteBuffer mem = store.data().memory.buffer(store);
                    int bufPtr = params[4].i32();
                    // filestat: dev(8) ino(8) filetype(1) nlink(8) size(8) atim(8) mtim(8) ctim(8) = 64 bytes
                    for (int i = 0; i < 64; i++) mem.put(bufPtr + i, (byte) 0);
                    try {
                        byte filetype = Files.isDirectory(p) ? (byte) 3 : (byte) 4;
                        mem.put(bufPtr + 16, filetype);
                        mem.putLong(bufPtr + 32, Files.size(p));
                    } catch (IOException ignored) {}
                    results[0] = Val.fromI32(ERRNO_SUCCESS);
                });

        // fd_readdir(fd, buf_ptr, buf_len, cookie, bufused_ptr) -> errno
        addFunc(store, funcs, funcMap, "fd_readdir",
                new Type[]{Type.I32, Type.I32, Type.I32, Type.I64, Type.I32}, new Type[]{Type.I32},
                (caller, params, results) -> {
                    int fd = params[0].i32();
                    int bufPtr = params[1].i32();
                    int bufLen = params[2].i32();
                    long cookie = params[3].i64();
                    int bufusedPtr = params[4].i32();

                    WasiFileDescriptor desc = store.data().fdTable.get(fd);
                    if (!(desc instanceof DirFd dirFd)) {
                        results[0] = Val.fromI32(ERRNO_BADF);
                        return;
                    }

                    ByteBuffer mem = store.data().memory.buffer(store);
                    mem.order(ByteOrder.LITTLE_ENDIAN);

                    java.util.List<DirFd.DirEntry> entries = dirFd.readDir(cookie);
                    int offset = 0;
                    long nextCookie = cookie;

                    for (DirFd.DirEntry entry : entries) {
                        nextCookie++;
                        byte[] nameBytes = entry.name().getBytes(StandardCharsets.UTF_8);
                        // dirent header: d_next(8) + d_ino(8) + d_namlen(4) + d_type(1) = 21 bytes
                        // followed by name bytes (NOT null-terminated)
                        // Align to 8 bytes? WASI spec doesn't require it for the packed format
                        int entrySize = 24 + nameBytes.length; // 24 = padded header

                        if (offset + entrySize > bufLen) {
                            break; // buffer full
                        }

                        int pos = bufPtr + offset;
                        mem.putLong(pos, nextCookie);           // d_next
                        mem.putLong(pos + 8, entry.inode());    // d_ino
                        mem.putInt(pos + 16, nameBytes.length); // d_namlen
                        mem.put(pos + 20, entry.type());        // d_type
                        // bytes 21-23: padding
                        for (int i = 0; i < nameBytes.length; i++) {
                            mem.put(pos + 24 + i, nameBytes[i]);
                        }
                        offset += entrySize;
                    }

                    mem.putInt(bufusedPtr, offset);
                    results[0] = Val.fromI32(ERRNO_SUCCESS);
                });
    }

    // --- Helper: scatter-gather fd_write ---

    private static int wasifdWrite(Store<WasiState> store, int fd, int iovsPtr, int iovsLen, int nwrittenPtr) {
        WasiFileDescriptor desc = store.data().fdTable.get(fd);
        if (desc == null) return ERRNO_BADF;

        ByteBuffer mem = store.data().memory.buffer(store);
        mem.order(ByteOrder.LITTLE_ENDIAN);
        int total = 0;

        for (int i = 0; i < iovsLen; i++) {
            int iovAddr = iovsPtr + i * 8;
            int bufPtr = mem.getInt(iovAddr);
            int bufLen = mem.getInt(iovAddr + 4);

            byte[] data = new byte[bufLen];
            for (int j = 0; j < bufLen; j++) data[j] = mem.get(bufPtr + j);

            try {
                int written = desc.write(data, 0, bufLen);
                if (written < 0) return ERRNO_BADF;
                total += written;
            } catch (IOException e) {
                return ERRNO_BADF;
            }
        }

        mem.putInt(nwrittenPtr, total);
        return ERRNO_SUCCESS;
    }

    // --- Helper: scatter-gather fd_read ---

    private static int wasifdRead(Store<WasiState> store, int fd, int iovsPtr, int iovsLen, int nreadPtr) {
        WasiFileDescriptor desc = store.data().fdTable.get(fd);
        if (desc == null) return ERRNO_BADF;

        ByteBuffer mem = store.data().memory.buffer(store);
        mem.order(ByteOrder.LITTLE_ENDIAN);
        int total = 0;

        for (int i = 0; i < iovsLen; i++) {
            int iovAddr = iovsPtr + i * 8;
            int bufPtr = mem.getInt(iovAddr);
            int bufLen = mem.getInt(iovAddr + 4);

            byte[] data = new byte[bufLen];
            try {
                int nread = desc.read(data, 0, bufLen);
                if (nread <= 0) break; // EOF or no data
                for (int j = 0; j < nread; j++) mem.put(bufPtr + j, data[j]);
                total += nread;
                if (nread < bufLen) break; // short read
            } catch (IOException e) {
                return ERRNO_BADF;
            }
        }

        mem.putInt(nreadPtr, total);
        return ERRNO_SUCCESS;
    }

    // --- Helpers ---

    private static String readString(Store<WasiState> store, int ptr, int len) {
        ByteBuffer mem = store.data().memory.buffer(store);
        byte[] bytes = new byte[len];
        for (int i = 0; i < len; i++) bytes[i] = mem.get(ptr + i);
        return new String(bytes, StandardCharsets.UTF_8);
    }

    @FunctionalInterface
    interface WasiCallback {
        void call(Object caller, Val[] params, Val[] results);
    }

    private static void addFunc(Store<WasiState> store, List<Func> funcs,
                                 java.util.Map<String, Extern> funcMap,
                                 String name, Type[] params, Type[] results,
                                 WasiCallback callback) {
        Func f = new Func(store, new FuncType(params, results),
                (caller, p, r) -> callback.call(caller, p, r));
        funcs.add(f);
        funcMap.put(WASI_NS + "::" + name, Extern.fromFunc(f));
        funcMap.put(name, Extern.fromFunc(f));
    }

    // --- Socket host function registration ---

    private static final String ENV_NS = "env";

    private static void addEnvFunc(Store<WasiState> store, List<Func> funcs,
                                    java.util.Map<String, Extern> funcMap,
                                    String name, Type[] params, Type[] results,
                                    WasiCallback callback) {
        Func f = new Func(store, new FuncType(params, results),
                (caller, p, r) -> callback.call(caller, p, r));
        funcs.add(f);
        funcMap.put(ENV_NS + "::" + name, Extern.fromFunc(f));
        funcMap.put(name, Extern.fromFunc(f));
    }

    /**
     * Register POSIX socket host functions for a child WASI process.
     * These are imported under the "env" module by ecm-host-abi's socket.rs.
     */
    public static void registerSocketFunctions(Store<WasiState> store, List<Func> funcs,
                                                java.util.Map<String, Extern> funcMap,
                                                NetIpcBridge bridge, int sessionId) {
        FdTable fdTable = store.data().fdTable;

        // sock_socket(domain: i32, type: i32, protocol: i32) -> i32 (fd or -1)
        addEnvFunc(store, funcs, funcMap, "sock_socket",
                new Type[]{Type.I32, Type.I32, Type.I32}, new Type[]{Type.I32},
                (caller, params, results) -> {
                    int domain = params[0].i32();
                    int sockType = params[1].i32();
                    int protocol = params[2].i32();
                    // Args: [domain: i32, type: i32, protocol: i32]
                    byte[] args = new byte[12];
                    ByteBuffer ab = ByteBuffer.wrap(args).order(ByteOrder.LITTLE_ENDIAN);
                    ab.putInt(0, domain);
                    ab.putInt(4, sockType);
                    ab.putInt(8, protocol);
                    byte[] resp = bridge.callBlocking(sessionId, SocketFd.SOCK_SOCKET, args, 5000);
                    int kernelSockId = SocketFd.decodeI32(resp, 0);
                    if (kernelSockId < 0) {
                        results[0] = Val.fromI32(-1);
                        return;
                    }
                    // Create SocketFd and allocate in child's FdTable
                    SocketFd sockFd = new SocketFd(kernelSockId, sessionId, bridge);
                    int fd = fdTable.allocate(sockFd);
                    results[0] = Val.fromI32(fd);
                });

        // sock_bind(fd: i32, addr_ptr: i32, addr_len: i32) -> i32
        addEnvFunc(store, funcs, funcMap, "sock_bind",
                new Type[]{Type.I32, Type.I32, Type.I32}, new Type[]{Type.I32},
                (caller, params, results) -> {
                    int fd = params[0].i32();
                    int addrPtr = params[1].i32();
                    int addrLen = params[2].i32();
                    WasiFileDescriptor desc = fdTable.get(fd);
                    if (!(desc instanceof SocketFd sock)) { results[0] = Val.fromI32(-1); return; }
                    Memory mem = store.data().memory;
                    ByteBuffer mb = mem.buffer(store);
                    byte[] addr = new byte[Math.min(addrLen, 16)];
                    mb.position(addrPtr);
                    mb.get(addr);
                    // Args: [sock_id: i32, addr_bytes...]
                    byte[] args = new byte[4 + addr.length];
                    ByteBuffer.wrap(args).order(ByteOrder.LITTLE_ENDIAN).putInt(0, sock.getKernelSocketId());
                    System.arraycopy(addr, 0, args, 4, addr.length);
                    byte[] resp = bridge.callBlocking(sessionId, SocketFd.SOCK_BIND, args, 5000);
                    results[0] = Val.fromI32(SocketFd.decodeI32(resp, 0));
                });

        // sock_connect(fd: i32, addr_ptr: i32, addr_len: i32) -> i32
        addEnvFunc(store, funcs, funcMap, "sock_connect",
                new Type[]{Type.I32, Type.I32, Type.I32}, new Type[]{Type.I32},
                (caller, params, results) -> {
                    int fd = params[0].i32();
                    int addrPtr = params[1].i32();
                    int addrLen = params[2].i32();
                    WasiFileDescriptor desc = fdTable.get(fd);
                    if (!(desc instanceof SocketFd sock)) { results[0] = Val.fromI32(-1); return; }
                    Memory mem = store.data().memory;
                    ByteBuffer mb = mem.buffer(store);
                    byte[] addr = new byte[Math.min(addrLen, 16)];
                    mb.position(addrPtr);
                    mb.get(addr);
                    byte[] args = new byte[4 + addr.length];
                    ByteBuffer.wrap(args).order(ByteOrder.LITTLE_ENDIAN).putInt(0, sock.getKernelSocketId());
                    System.arraycopy(addr, 0, args, 4, addr.length);
                    byte[] resp = bridge.callBlocking(sessionId, SocketFd.SOCK_CONNECT, args, 10000);
                    results[0] = Val.fromI32(SocketFd.decodeI32(resp, 0));
                });

        // sock_listen(fd: i32, backlog: i32) -> i32
        addEnvFunc(store, funcs, funcMap, "sock_listen",
                new Type[]{Type.I32, Type.I32}, new Type[]{Type.I32},
                (caller, params, results) -> {
                    int fd = params[0].i32();
                    int backlog = params[1].i32();
                    WasiFileDescriptor desc = fdTable.get(fd);
                    if (!(desc instanceof SocketFd sock)) { results[0] = Val.fromI32(-1); return; }
                    byte[] args = new byte[8];
                    ByteBuffer ab = ByteBuffer.wrap(args).order(ByteOrder.LITTLE_ENDIAN);
                    ab.putInt(0, sock.getKernelSocketId());
                    ab.putInt(4, backlog);
                    byte[] resp = bridge.callBlocking(sessionId, SocketFd.SOCK_LISTEN, args, 5000);
                    results[0] = Val.fromI32(SocketFd.decodeI32(resp, 0));
                });

        // sock_accept(fd: i32, addr_ptr: i32, addr_len_ptr: i32) -> i32 (new fd or -1)
        addEnvFunc(store, funcs, funcMap, "sock_accept",
                new Type[]{Type.I32, Type.I32, Type.I32}, new Type[]{Type.I32},
                (caller, params, results) -> {
                    int fd = params[0].i32();
                    int addrPtr = params[1].i32();
                    int addrLenPtr = params[2].i32();
                    WasiFileDescriptor desc = fdTable.get(fd);
                    if (!(desc instanceof SocketFd sock)) { results[0] = Val.fromI32(-1); return; }
                    byte[] args = new byte[4];
                    ByteBuffer.wrap(args).order(ByteOrder.LITTLE_ENDIAN).putInt(0, sock.getKernelSocketId());
                    byte[] resp = bridge.callBlocking(sessionId, SocketFd.SOCK_ACCEPT, args, 30000);
                    if (resp.length < 4) { results[0] = Val.fromI32(-1); return; }
                    int newKernelSockId = SocketFd.decodeI32(resp, 0);
                    if (newKernelSockId < 0) { results[0] = Val.fromI32(newKernelSockId); return; }
                    // Create new SocketFd for accepted connection
                    SocketFd newSock = new SocketFd(newKernelSockId, sessionId, bridge);
                    int newFd = fdTable.allocate(newSock);
                    // Write peer address if provided (bytes 4+ in response)
                    if (resp.length > 4 && addrPtr != 0) {
                        Memory mem = store.data().memory;
                        ByteBuffer mb = mem.buffer(store);
                        int addrBytes = Math.min(resp.length - 4, 16);
                        mb.position(addrPtr);
                        mb.put(resp, 4, addrBytes);
                        if (addrLenPtr != 0) {
                            mb.position(addrLenPtr);
                            mb.putInt(addrBytes);
                        }
                    }
                    results[0] = Val.fromI32(newFd);
                });

        // sock_send(fd: i32, buf_ptr: i32, buf_len: i32, flags: i32) -> i32
        addEnvFunc(store, funcs, funcMap, "sock_send",
                new Type[]{Type.I32, Type.I32, Type.I32, Type.I32}, new Type[]{Type.I32},
                (caller, params, results) -> {
                    int fd = params[0].i32();
                    int bufPtr = params[1].i32();
                    int bufLen = params[2].i32();
                    WasiFileDescriptor desc = fdTable.get(fd);
                    if (!(desc instanceof SocketFd sock)) { results[0] = Val.fromI32(-1); return; }
                    Memory mem = store.data().memory;
                    ByteBuffer mb = mem.buffer(store);
                    byte[] data = new byte[bufLen];
                    mb.position(bufPtr);
                    mb.get(data);
                    // Args: [sock_id: i32, data_len: u16, data...]
                    byte[] args = new byte[4 + 2 + bufLen];
                    ByteBuffer ab = ByteBuffer.wrap(args).order(ByteOrder.LITTLE_ENDIAN);
                    ab.putInt(0, sock.getKernelSocketId());
                    ab.putShort(4, (short) bufLen);
                    System.arraycopy(data, 0, args, 6, bufLen);
                    byte[] resp = bridge.callBlocking(sessionId, SocketFd.SOCK_SEND, args, 30000);
                    results[0] = Val.fromI32(SocketFd.decodeI32(resp, 0));
                });

        // sock_recv(fd: i32, buf_ptr: i32, buf_len: i32, flags: i32) -> i32
        addEnvFunc(store, funcs, funcMap, "sock_recv",
                new Type[]{Type.I32, Type.I32, Type.I32, Type.I32}, new Type[]{Type.I32},
                (caller, params, results) -> {
                    int fd = params[0].i32();
                    int bufPtr = params[1].i32();
                    int bufLen = params[2].i32();
                    int flags = params[3].i32();
                    WasiFileDescriptor desc = fdTable.get(fd);
                    if (!(desc instanceof SocketFd sock)) { results[0] = Val.fromI32(-1); return; }
                    byte[] args = new byte[12];
                    ByteBuffer ab = ByteBuffer.wrap(args).order(ByteOrder.LITTLE_ENDIAN);
                    ab.putInt(0, sock.getKernelSocketId());
                    ab.putInt(4, bufLen);
                    ab.putInt(8, flags);
                    byte[] resp = bridge.callBlocking(sessionId, SocketFd.SOCK_RECV, args, 30000);
                    if (resp.length == 0) { results[0] = Val.fromI32(0); return; }
                    if (resp.length == 4) {
                        int val = SocketFd.decodeI32(resp, 0);
                        if (val <= 0) { results[0] = Val.fromI32(val); return; }
                    }
                    // Response is raw data bytes
                    int copyLen = Math.min(resp.length, bufLen);
                    Memory mem = store.data().memory;
                    ByteBuffer mb = mem.buffer(store);
                    mb.position(bufPtr);
                    mb.put(resp, 0, copyLen);
                    results[0] = Val.fromI32(copyLen);
                });

        // sock_sendto(fd, buf_ptr, buf_len, flags, addr_ptr, addr_len) -> i32
        addEnvFunc(store, funcs, funcMap, "sock_sendto",
                new Type[]{Type.I32, Type.I32, Type.I32, Type.I32, Type.I32, Type.I32}, new Type[]{Type.I32},
                (caller, params, results) -> {
                    int fd = params[0].i32();
                    int bufPtr = params[1].i32();
                    int bufLen = params[2].i32();
                    int addrPtr = params[4].i32();
                    int addrLen = params[5].i32();
                    WasiFileDescriptor desc = fdTable.get(fd);
                    if (!(desc instanceof SocketFd sock)) { results[0] = Val.fromI32(-1); return; }
                    Memory mem = store.data().memory;
                    ByteBuffer mb = mem.buffer(store);
                    byte[] data = new byte[bufLen];
                    mb.position(bufPtr);
                    mb.get(data);
                    byte[] addr = new byte[Math.min(addrLen, 16)];
                    mb.position(addrPtr);
                    mb.get(addr);
                    // Args: [sock_id: i32, addr_len: u16, addr..., data_len: u16, data...]
                    byte[] args = new byte[4 + 2 + addr.length + 2 + bufLen];
                    ByteBuffer ab = ByteBuffer.wrap(args).order(ByteOrder.LITTLE_ENDIAN);
                    ab.putInt(0, sock.getKernelSocketId());
                    ab.putShort(4, (short) addr.length);
                    System.arraycopy(addr, 0, args, 6, addr.length);
                    int dataOff = 6 + addr.length;
                    ab.putShort(dataOff, (short) bufLen);
                    System.arraycopy(data, 0, args, dataOff + 2, bufLen);
                    byte[] resp = bridge.callBlocking(sessionId, SocketFd.SOCK_SENDTO, args, 30000);
                    results[0] = Val.fromI32(SocketFd.decodeI32(resp, 0));
                });

        // sock_recvfrom(fd, buf_ptr, buf_len, flags, addr_ptr, addr_len_ptr) -> i32
        addEnvFunc(store, funcs, funcMap, "sock_recvfrom",
                new Type[]{Type.I32, Type.I32, Type.I32, Type.I32, Type.I32, Type.I32}, new Type[]{Type.I32},
                (caller, params, results) -> {
                    int fd = params[0].i32();
                    int bufPtr = params[1].i32();
                    int bufLen = params[2].i32();
                    int addrPtr = params[4].i32();
                    int addrLenPtr = params[5].i32();
                    WasiFileDescriptor desc = fdTable.get(fd);
                    if (!(desc instanceof SocketFd sock)) { results[0] = Val.fromI32(-1); return; }
                    byte[] args = new byte[12];
                    ByteBuffer ab = ByteBuffer.wrap(args).order(ByteOrder.LITTLE_ENDIAN);
                    ab.putInt(0, sock.getKernelSocketId());
                    ab.putInt(4, bufLen);
                    ab.putInt(8, 0); // flags
                    byte[] resp = bridge.callBlocking(sessionId, SocketFd.SOCK_RECVFROM, args, 30000);
                    if (resp.length < 4) { results[0] = Val.fromI32(-1); return; }
                    // Response: [data_len: i32, addr_bytes (16), data_bytes...]
                    int dataLen = SocketFd.decodeI32(resp, 0);
                    if (dataLen <= 0) { results[0] = Val.fromI32(dataLen); return; }
                    Memory mem = store.data().memory;
                    ByteBuffer mb = mem.buffer(store);
                    // Write source address (bytes 4..20)
                    if (addrPtr != 0 && resp.length >= 20) {
                        mb.position(addrPtr);
                        mb.put(resp, 4, 16);
                        if (addrLenPtr != 0) {
                            mb.position(addrLenPtr);
                            mb.putInt(16);
                        }
                    }
                    // Write data (bytes 20+)
                    int dataStart = 20;
                    int copyLen = Math.min(dataLen, Math.min(resp.length - dataStart, bufLen));
                    if (copyLen > 0) {
                        mb = mem.buffer(store);
                        mb.position(bufPtr);
                        mb.put(resp, dataStart, copyLen);
                    }
                    results[0] = Val.fromI32(copyLen);
                });

        // sock_setsockopt(fd, level, optname, optval_ptr, optlen) -> i32
        addEnvFunc(store, funcs, funcMap, "sock_setsockopt",
                new Type[]{Type.I32, Type.I32, Type.I32, Type.I32, Type.I32}, new Type[]{Type.I32},
                (caller, params, results) -> {
                    int fd = params[0].i32();
                    int level = params[1].i32();
                    int optname = params[2].i32();
                    int optvalPtr = params[3].i32();
                    int optlen = params[4].i32();
                    WasiFileDescriptor desc = fdTable.get(fd);
                    if (!(desc instanceof SocketFd sock)) { results[0] = Val.fromI32(-1); return; }
                    Memory mem = store.data().memory;
                    ByteBuffer mb = mem.buffer(store);
                    byte[] optval = new byte[Math.min(optlen, 64)];
                    mb.position(optvalPtr);
                    mb.get(optval);
                    byte[] args = new byte[12 + optval.length];
                    ByteBuffer ab = ByteBuffer.wrap(args).order(ByteOrder.LITTLE_ENDIAN);
                    ab.putInt(0, sock.getKernelSocketId());
                    ab.putInt(4, level);
                    ab.putInt(8, optname);
                    System.arraycopy(optval, 0, args, 12, optval.length);
                    byte[] resp = bridge.callBlocking(sessionId, SocketFd.SOCK_SETSOCKOPT, args, 5000);
                    results[0] = Val.fromI32(SocketFd.decodeI32(resp, 0));
                });

        // sock_getsockname(fd, addr_ptr, addr_len_ptr) -> i32
        addEnvFunc(store, funcs, funcMap, "sock_getsockname",
                new Type[]{Type.I32, Type.I32, Type.I32}, new Type[]{Type.I32},
                (caller, params, results) -> {
                    int fd = params[0].i32();
                    int addrPtr = params[1].i32();
                    int addrLenPtr = params[2].i32();
                    WasiFileDescriptor desc = fdTable.get(fd);
                    if (!(desc instanceof SocketFd sock)) { results[0] = Val.fromI32(-1); return; }
                    byte[] args = SocketFd.encodeI32(sock.getKernelSocketId());
                    byte[] resp = bridge.callBlocking(sessionId, SocketFd.SOCK_GETSOCKNAME, args, 5000);
                    if (resp.length < 16) { results[0] = Val.fromI32(-1); return; }
                    Memory mem = store.data().memory;
                    ByteBuffer mb = mem.buffer(store);
                    mb.position(addrPtr);
                    mb.put(resp, 0, 16);
                    if (addrLenPtr != 0) {
                        mb.position(addrLenPtr);
                        mb.putInt(16);
                    }
                    results[0] = Val.fromI32(0);
                });

        // sock_getpeername(fd, addr_ptr, addr_len_ptr) -> i32
        addEnvFunc(store, funcs, funcMap, "sock_getpeername",
                new Type[]{Type.I32, Type.I32, Type.I32}, new Type[]{Type.I32},
                (caller, params, results) -> {
                    int fd = params[0].i32();
                    int addrPtr = params[1].i32();
                    int addrLenPtr = params[2].i32();
                    WasiFileDescriptor desc = fdTable.get(fd);
                    if (!(desc instanceof SocketFd sock)) { results[0] = Val.fromI32(-1); return; }
                    byte[] args = SocketFd.encodeI32(sock.getKernelSocketId());
                    byte[] resp = bridge.callBlocking(sessionId, SocketFd.SOCK_GETPEERNAME, args, 5000);
                    if (resp.length < 16) { results[0] = Val.fromI32(-1); return; }
                    Memory mem = store.data().memory;
                    ByteBuffer mb = mem.buffer(store);
                    mb.position(addrPtr);
                    mb.put(resp, 0, 16);
                    if (addrLenPtr != 0) {
                        mb.position(addrLenPtr);
                        mb.putInt(16);
                    }
                    results[0] = Val.fromI32(0);
                });

        // sock_shutdown(fd, how) -> i32
        addEnvFunc(store, funcs, funcMap, "sock_shutdown",
                new Type[]{Type.I32, Type.I32}, new Type[]{Type.I32},
                (caller, params, results) -> {
                    int fd = params[0].i32();
                    int how = params[1].i32();
                    WasiFileDescriptor desc = fdTable.get(fd);
                    if (!(desc instanceof SocketFd sock)) { results[0] = Val.fromI32(-1); return; }
                    byte[] args = new byte[8];
                    ByteBuffer ab = ByteBuffer.wrap(args).order(ByteOrder.LITTLE_ENDIAN);
                    ab.putInt(0, sock.getKernelSocketId());
                    ab.putInt(4, how);
                    byte[] resp = bridge.callBlocking(sessionId, SocketFd.SOCK_SHUTDOWN, args, 5000);
                    results[0] = Val.fromI32(SocketFd.decodeI32(resp, 0));
                });

        // sock_getaddrinfo(host_ptr, host_len, result_ptr, result_len) -> i32
        addEnvFunc(store, funcs, funcMap, "sock_getaddrinfo",
                new Type[]{Type.I32, Type.I32, Type.I32, Type.I32}, new Type[]{Type.I32},
                (caller, params, results) -> {
                    int hostPtr = params[0].i32();
                    int hostLen = params[1].i32();
                    int resultPtr = params[2].i32();
                    int resultLen = params[3].i32();
                    String host = readString(store, hostPtr, hostLen);
                    byte[] hostBytes = host.getBytes(StandardCharsets.UTF_8);
                    // Args: [host_len: u16, host_bytes...]
                    byte[] args = new byte[2 + hostBytes.length];
                    ByteBuffer.wrap(args).order(ByteOrder.LITTLE_ENDIAN).putShort(0, (short) hostBytes.length);
                    System.arraycopy(hostBytes, 0, args, 2, hostBytes.length);
                    byte[] resp = bridge.callBlocking(sessionId, SocketFd.SOCK_GETADDRINFO, args, 10000);
                    if (resp.length < 4) { results[0] = Val.fromI32(-1); return; }
                    // Response is a sockaddr_in (16 bytes) or error
                    int copyLen = Math.min(resp.length, resultLen);
                    Memory mem = store.data().memory;
                    ByteBuffer mb = mem.buffer(store);
                    mb.position(resultPtr);
                    mb.put(resp, 0, copyLen);
                    results[0] = Val.fromI32(copyLen);
                });

        // get_time_ms() -> i64  (millisecond wall clock for child processes)
        addEnvFunc(store, funcs, funcMap, "get_time_ms",
                new Type[]{}, new Type[]{Type.I64},
                (caller, params, results) -> {
                    results[0] = Val.fromI64(System.currentTimeMillis());
                });
    }

    /**
     * Exception thrown by proc_exit() to terminate the child process.
     */
    public static class WasiExitException extends RuntimeException {
        public final int exitCode;
        public WasiExitException(int exitCode) {
            super("proc_exit(" + exitCode + ")");
            this.exitCode = exitCode;
        }
    }
}
