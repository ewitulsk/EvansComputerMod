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
     *
     * @param childBridge bridge to kernel-side host operations (redstone, peripherals, sleep);
     *                    may be null in unit tests, in which case those functions return errors
     */
    public static void register(Store<WasiState> store, List<Func> funcs,
                                 java.util.Map<String, Extern> funcMap,
                                 ChildHostBridge childBridge) {
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
                    int closeFd = params[0].i32();
                    WasiFileDescriptor closeDesc = store.data().fdTable.get(closeFd);
                    if (closeFd > 2) {
                        EvansComputerMod.LOGGER.debug("WASI fd_close: fd={} ({})", closeFd,
                                closeDesc != null ? closeDesc.getClass().getSimpleName() : "null");
                    }
                    store.data().fdTable.close(closeFd);
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
                            mem.order(ByteOrder.LITTLE_ENDIAN);
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
                    mem.order(ByteOrder.LITTLE_ENDIAN);
                    // fdstat: filetype(1), fdflags(2), rights_base(8), rights_inheriting(8) = 24 bytes
                    for (int i = 0; i < 24; i++) mem.put(bufPtr + i, (byte) 0);
                    if (fd <= 2) {
                        mem.put(bufPtr, (byte) 2); // FILETYPE_CHARACTER_DEVICE
                    } else if (fd == 3) {
                        mem.put(bufPtr, (byte) 3); // FILETYPE_DIRECTORY
                    } else {
                        mem.put(bufPtr, (byte) 4); // FILETYPE_REGULAR_FILE
                    }
                    // Set all rights (full 64-bit mask so all WASI rights bits are set)
                    mem.putLong(bufPtr + 8, -1L);
                    mem.putLong(bufPtr + 16, -1L);
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
                    mem.order(ByteOrder.LITTLE_ENDIAN);
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
                    mem.order(ByteOrder.LITTLE_ENDIAN);
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
                        EvansComputerMod.LOGGER.debug("WASI path_open: rejected path (traversal): {}", pathStr);
                        results[0] = Val.fromI32(ERRNO_NOENT);
                        return;
                    }

                    Path filePath = store.data().storagePath.resolve(pathStr).normalize();
                    if (!filePath.startsWith(store.data().storagePath)) {
                        EvansComputerMod.LOGGER.debug("WASI path_open: rejected path (escape): {}", pathStr);
                        results[0] = Val.fromI32(ERRNO_NOENT);
                        return;
                    }

                    boolean create = (oflags & 1) != 0; // OFLAGS_CREAT
                    boolean trunc = (oflags & 8) != 0;   // OFLAGS_TRUNC
                    boolean append = (fdflags & 1) != 0;  // FDFLAGS_APPEND

                    EvansComputerMod.LOGGER.debug("WASI path_open: path='{}' oflags={} create={} trunc={} append={} resolved={}",
                            pathStr, oflags, create, trunc, append, filePath);

                    if (!Files.exists(filePath) && !create) {
                        EvansComputerMod.LOGGER.debug("WASI path_open: NOENT (file doesn't exist and no O_CREAT)");
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
                            EvansComputerMod.LOGGER.debug("WASI path_open: opened dir fd={}", newFd);
                        } else {
                            // Regular file
                            if (create && !Files.exists(filePath)) {
                                Files.createDirectories(filePath.getParent());
                                Files.createFile(filePath);
                                EvansComputerMod.LOGGER.debug("WASI path_open: created new file");
                            }
                            VfsFileFd vfs = new VfsFileFd(filePath, true, true, append);
                            if (trunc) vfs.truncate();
                            int newFd = store.data().fdTable.allocate(vfs);
                            mem.putInt(fdOutPtr, newFd);
                            EvansComputerMod.LOGGER.debug("WASI path_open: opened file fd={} size={}", newFd, Files.size(filePath));
                        }
                        results[0] = Val.fromI32(ERRNO_SUCCESS);
                    } catch (IOException e) {
                        EvansComputerMod.LOGGER.error("WASI path_open: IOException for {}", pathStr, e);
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
                    mem.order(ByteOrder.LITTLE_ENDIAN);
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

        // === Kernel-style file_* host functions (used by ecm_host_abi::fs) ===
        // These mirror ComputerInstance's hostFile* methods but operate on the
        // child process's storagePath. Registered under both bare and "env::"
        // qualified names so the import resolver finds them either way.

        // file_read(path_ptr, path_len, buf_ptr, buf_len) -> bytes_read or -1
        addEnvFunc(store, funcs, funcMap, "file_read",
                new Type[]{Type.I32, Type.I32, Type.I32, Type.I32}, new Type[]{Type.I32},
                (caller, params, results) -> {
                    String path = readString(store, params[0].i32(), params[1].i32());
                    int bufPtr = params[2].i32();
                    int bufLen = params[3].i32();
                    Path filePath = resolveChildPath(store, path);
                    if (filePath == null || !Files.exists(filePath) || Files.isDirectory(filePath)) {
                        results[0] = Val.fromI32(-1);
                        return;
                    }
                    try {
                        byte[] data = Files.readAllBytes(filePath);
                        int n = Math.min(data.length, bufLen);
                        ByteBuffer mem = store.data().memory.buffer(store);
                        for (int i = 0; i < n; i++) mem.put(bufPtr + i, data[i]);
                        results[0] = Val.fromI32(n);
                    } catch (IOException e) {
                        results[0] = Val.fromI32(-1);
                    }
                });

        // file_write(path_ptr, path_len, data_ptr, data_len) -> bytes_written or -1
        addEnvFunc(store, funcs, funcMap, "file_write",
                new Type[]{Type.I32, Type.I32, Type.I32, Type.I32}, new Type[]{Type.I32},
                (caller, params, results) -> {
                    String path = readString(store, params[0].i32(), params[1].i32());
                    int dataPtr = params[2].i32();
                    int dataLen = params[3].i32();
                    Path filePath = resolveChildPath(store, path);
                    if (filePath == null || dataLen < 0 || dataLen > 1024 * 1024) {
                        results[0] = Val.fromI32(-1);
                        return;
                    }
                    try {
                        ByteBuffer mem = store.data().memory.buffer(store);
                        byte[] data = new byte[dataLen];
                        for (int i = 0; i < dataLen; i++) data[i] = mem.get(dataPtr + i);
                        Path parent = filePath.getParent();
                        if (parent != null && !Files.exists(parent)) {
                            Files.createDirectories(parent);
                        }
                        Files.write(filePath, data, java.nio.file.StandardOpenOption.CREATE,
                                java.nio.file.StandardOpenOption.TRUNCATE_EXISTING);
                        results[0] = Val.fromI32(dataLen);
                    } catch (IOException e) {
                        results[0] = Val.fromI32(-1);
                    }
                });

        // file_size(path_ptr, path_len) -> size or -1
        addEnvFunc(store, funcs, funcMap, "file_size",
                new Type[]{Type.I32, Type.I32}, new Type[]{Type.I32},
                (caller, params, results) -> {
                    String path = readString(store, params[0].i32(), params[1].i32());
                    Path filePath = resolveChildPath(store, path);
                    if (filePath == null || !Files.exists(filePath)) {
                        results[0] = Val.fromI32(-1);
                        return;
                    }
                    try {
                        results[0] = Val.fromI32((int) Files.size(filePath));
                    } catch (IOException e) {
                        results[0] = Val.fromI32(-1);
                    }
                });

        // file_exists(path_ptr, path_len) -> 1 if exists, 0 if not
        addEnvFunc(store, funcs, funcMap, "file_exists",
                new Type[]{Type.I32, Type.I32}, new Type[]{Type.I32},
                (caller, params, results) -> {
                    String path = readString(store, params[0].i32(), params[1].i32());
                    Path filePath = resolveChildPath(store, path);
                    results[0] = Val.fromI32(filePath != null && Files.exists(filePath) ? 1 : 0);
                });

        // file_delete(path_ptr, path_len) -> 1 on success, 0 on failure
        addEnvFunc(store, funcs, funcMap, "file_delete",
                new Type[]{Type.I32, Type.I32}, new Type[]{Type.I32},
                (caller, params, results) -> {
                    String path = readString(store, params[0].i32(), params[1].i32());
                    Path filePath = resolveChildPath(store, path);
                    if (filePath == null || !Files.exists(filePath)) {
                        results[0] = Val.fromI32(0);
                        return;
                    }
                    try {
                        Files.delete(filePath);
                        results[0] = Val.fromI32(1);
                    } catch (IOException e) {
                        results[0] = Val.fromI32(0);
                    }
                });

        // file_mkdir(path_ptr, path_len) -> 0 on success, -1 on error
        addEnvFunc(store, funcs, funcMap, "file_mkdir",
                new Type[]{Type.I32, Type.I32}, new Type[]{Type.I32},
                (caller, params, results) -> {
                    String path = readString(store, params[0].i32(), params[1].i32());
                    Path dirPath = resolveChildPath(store, path);
                    if (dirPath == null) {
                        results[0] = Val.fromI32(-1);
                        return;
                    }
                    try {
                        Files.createDirectories(dirPath);
                        results[0] = Val.fromI32(0);
                    } catch (IOException e) {
                        results[0] = Val.fromI32(-1);
                    }
                });

        // file_is_dir(path_ptr, path_len) -> 1 if directory, 0 if not
        addEnvFunc(store, funcs, funcMap, "file_is_dir",
                new Type[]{Type.I32, Type.I32}, new Type[]{Type.I32},
                (caller, params, results) -> {
                    String path = readString(store, params[0].i32(), params[1].i32());
                    if (path == null || path.isEmpty()) {
                        results[0] = Val.fromI32(1); // root
                        return;
                    }
                    Path filePath = resolveChildPath(store, path);
                    results[0] = Val.fromI32(filePath != null && Files.isDirectory(filePath) ? 1 : 0);
                });

        // file_list(buf_ptr, buf_len) -> bytes written (newline-separated filenames)
        addEnvFunc(store, funcs, funcMap, "file_list",
                new Type[]{Type.I32, Type.I32}, new Type[]{Type.I32},
                (caller, params, results) -> {
                    int bufPtr = params[0].i32();
                    int bufLen = params[1].i32();
                    Path root = store.data().storagePath;
                    if (!Files.exists(root)) {
                        results[0] = Val.fromI32(0);
                        return;
                    }
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
                        ByteBuffer mem = store.data().memory.buffer(store);
                        for (int i = 0; i < n; i++) mem.put(bufPtr + i, data[i]);
                        results[0] = Val.fromI32(n);
                    } catch (IOException e) {
                        results[0] = Val.fromI32(-1);
                    }
                });

        // file_list_dir(path_ptr, path_len, buf_ptr, buf_len) -> bytes written or -1
        addEnvFunc(store, funcs, funcMap, "file_list_dir",
                new Type[]{Type.I32, Type.I32, Type.I32, Type.I32}, new Type[]{Type.I32},
                (caller, params, results) -> {
                    String path = readString(store, params[0].i32(), params[1].i32());
                    int bufPtr = params[2].i32();
                    int bufLen = params[3].i32();
                    Path dirPath = (path == null || path.isEmpty())
                            ? store.data().storagePath
                            : resolveChildPath(store, path);
                    if (dirPath == null || !Files.isDirectory(dirPath)) {
                        results[0] = Val.fromI32(-1);
                        return;
                    }
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
                        ByteBuffer mem = store.data().memory.buffer(store);
                        for (int i = 0; i < n; i++) mem.put(bufPtr + i, data[i]);
                        results[0] = Val.fromI32(n);
                    } catch (IOException e) {
                        results[0] = Val.fromI32(-1);
                    }
                });

        // === Redstone host functions (delegated through ChildHostBridge) ===

        // redstone_set_output(side: i32, power: i32) -> i32
        addEnvFunc(store, funcs, funcMap, "redstone_set_output",
                new Type[]{Type.I32, Type.I32}, new Type[]{Type.I32},
                (caller, params, results) -> {
                    if (childBridge == null) { results[0] = Val.fromI32(-1); return; }
                    results[0] = Val.fromI32(childBridge.redstoneSetOutput(params[0].i32(), params[1].i32()));
                });

        // redstone_get_input(side: i32) -> i32
        addEnvFunc(store, funcs, funcMap, "redstone_get_input",
                new Type[]{Type.I32}, new Type[]{Type.I32},
                (caller, params, results) -> {
                    if (childBridge == null) { results[0] = Val.fromI32(0); return; }
                    results[0] = Val.fromI32(childBridge.redstoneGetInput(params[0].i32()));
                });

        // redstone_get_all_input(buf_ptr: i32) -> i32
        // Writes 6 little-endian i32 values (24 bytes) to the child's WASM memory.
        addEnvFunc(store, funcs, funcMap, "redstone_get_all_input",
                new Type[]{Type.I32}, new Type[]{Type.I32},
                (caller, params, results) -> {
                    if (childBridge == null) { results[0] = Val.fromI32(-1); return; }
                    int bufPtr = params[0].i32();
                    int[] vals = new int[6];
                    int rc = childBridge.redstoneGetAllInput(vals);
                    if (rc != 0) { results[0] = Val.fromI32(rc); return; }
                    ByteBuffer mem = store.data().memory.buffer(store);
                    mem.order(ByteOrder.LITTLE_ENDIAN);
                    for (int i = 0; i < 6; i++) {
                        mem.putInt(bufPtr + i * 4, vals[i]);
                    }
                    results[0] = Val.fromI32(0);
                });

        // === Peripheral host functions (delegated through ChildHostBridge) ===

        // peripheral_list(buf_ptr, buf_len) -> bytes_written or -1
        addEnvFunc(store, funcs, funcMap, "peripheral_list",
                new Type[]{Type.I32, Type.I32}, new Type[]{Type.I32},
                (caller, params, results) -> {
                    if (childBridge == null) { results[0] = Val.fromI32(-1); return; }
                    String json = childBridge.peripheralListJson();
                    results[0] = Val.fromI32(writeStringToChildMemory(store, json, params[0].i32(), params[1].i32()));
                });

        // peripheral_get_methods(name_ptr, name_len, buf_ptr, buf_len) -> bytes_written or -1
        addEnvFunc(store, funcs, funcMap, "peripheral_get_methods",
                new Type[]{Type.I32, Type.I32, Type.I32, Type.I32}, new Type[]{Type.I32},
                (caller, params, results) -> {
                    if (childBridge == null) { results[0] = Val.fromI32(-1); return; }
                    String name = readString(store, params[0].i32(), params[1].i32());
                    String json = childBridge.peripheralMethodsJson(name);
                    results[0] = Val.fromI32(writeStringToChildMemory(store, json, params[2].i32(), params[3].i32()));
                });

        // peripheral_call(name_ptr, name_len, method_ptr, method_len,
        //                 args_ptr, args_len, result_ptr, result_len) -> bytes_written or -1
        addEnvFunc(store, funcs, funcMap, "peripheral_call",
                new Type[]{Type.I32, Type.I32, Type.I32, Type.I32, Type.I32, Type.I32, Type.I32, Type.I32},
                new Type[]{Type.I32},
                (caller, params, results) -> {
                    if (childBridge == null) { results[0] = Val.fromI32(-1); return; }
                    String name = readString(store, params[0].i32(), params[1].i32());
                    String method = readString(store, params[2].i32(), params[3].i32());
                    String args = params[5].i32() > 0 ? readString(store, params[4].i32(), params[5].i32()) : "[]";
                    String resultJson = childBridge.peripheralCall(name, method, args);
                    results[0] = Val.fromI32(writeStringToChildMemory(store, resultJson, params[6].i32(), params[7].i32()));
                });

        // === Sleep / time ===

        // sleep_ms(milliseconds: i32) -> ()
        addEnvFunc(store, funcs, funcMap, "sleep_ms",
                new Type[]{Type.I32}, new Type[]{},
                (caller, params, results) -> {
                    if (childBridge != null) {
                        childBridge.sleepMs(params[0].i32());
                    }
                });

        // get_time_ms() -> i64
        addEnvFunc(store, funcs, funcMap, "get_time_ms",
                new Type[]{}, new Type[]{Type.I64},
                (caller, params, results) -> {
                    results[0] = Val.fromI64(System.currentTimeMillis());
                });

        // === Raw packet capture host functions (for tcpdump) ===

        // net_set_promiscuous_on(index: i32, enabled: i32) -> i32
        addEnvFunc(store, funcs, funcMap, "net_set_promiscuous_on",
                new Type[]{Type.I32, Type.I32}, new Type[]{Type.I32},
                (caller, params, results) -> {
                    if (childBridge == null) {
                        results[0] = Val.fromI32(-1);
                        return;
                    }
                    int index = params[0].i32();
                    int enabled = params[1].i32();
                    results[0] = Val.fromI32(childBridge.netSetPromiscuousOn(index, enabled));
                });

        // net_pcap_enable(index: i32, enabled: i32) -> i32
        addEnvFunc(store, funcs, funcMap, "net_pcap_enable",
                new Type[]{Type.I32, Type.I32}, new Type[]{Type.I32},
                (caller, params, results) -> {
                    if (childBridge == null) {
                        results[0] = Val.fromI32(-1);
                        return;
                    }
                    int index = params[0].i32();
                    int enabled = params[1].i32();
                    results[0] = Val.fromI32(childBridge.netPcapEnable(index, enabled));
                });

        // net_pcap_rx(index: i32, buf_ptr: i32, buf_len: i32) -> i32
        addEnvFunc(store, funcs, funcMap, "net_pcap_rx",
                new Type[]{Type.I32, Type.I32, Type.I32}, new Type[]{Type.I32},
                (caller, params, results) -> {
                    if (childBridge == null) {
                        results[0] = Val.fromI32(-1);
                        return;
                    }
                    int index = params[0].i32();
                    int bufPtr = params[1].i32();
                    int bufLen = params[2].i32();
                    if (bufLen <= 0) {
                        results[0] = Val.fromI32(-1);
                        return;
                    }

                    byte[] frame = childBridge.netPcapRx(index);
                    if (frame == null) {
                        results[0] = Val.fromI32(-1);
                        return;
                    }

                    int writeLen = Math.min(frame.length, bufLen);
                    ByteBuffer mem = store.data().memory.buffer(store);
                    for (int i = 0; i < writeLen; i++) {
                        mem.put(bufPtr + i, frame[i]);
                    }
                    results[0] = Val.fromI32(writeLen);
                });

        // === Video playback host functions (used by the `player` program) ===

        // video_open(path_ptr: i32, path_len: i32, target_w: i32, target_h: i32, format: i32) -> i32
        // Returns a handle (>= 1) or -1 on failure. format=0 indexed8, 1 rgba8888.
        addEnvFunc(store, funcs, funcMap, "video_open",
                new Type[]{Type.I32, Type.I32, Type.I32, Type.I32, Type.I32}, new Type[]{Type.I32},
                (caller, params, results) -> {
                    if (childBridge == null) { results[0] = Val.fromI32(-1); return; }
                    String path = readString(store, params[0].i32(), params[1].i32());
                    int tw = params[2].i32();
                    int th = params[3].i32();
                    int format = params[4].i32();
                    results[0] = Val.fromI32(childBridge.videoOpen(path, tw, th, format));
                });

        // video_get_info(handle: i32, out_ptr: i32) -> i32
        // Writes a 32-byte VideoInfo struct to the child's memory:
        //   u32 width, u32 height, u32 fps_num, u32 fps_den,
        //   u64 frame_count, u64 duration_ms
        // Returns 0 on success, -1 on unknown handle.
        addEnvFunc(store, funcs, funcMap, "video_get_info",
                new Type[]{Type.I32, Type.I32}, new Type[]{Type.I32},
                (caller, params, results) -> {
                    if (childBridge == null) { results[0] = Val.fromI32(-1); return; }
                    int handle = params[0].i32();
                    int outPtr = params[1].i32();
                    var info = childBridge.videoGetInfo(handle);
                    if (info == null) { results[0] = Val.fromI32(-1); return; }
                    ByteBuffer mem = store.data().memory.buffer(store);
                    mem.order(ByteOrder.LITTLE_ENDIAN);
                    mem.putInt(outPtr,      info.width);
                    mem.putInt(outPtr + 4,  info.height);
                    mem.putInt(outPtr + 8,  info.fpsNum);
                    mem.putInt(outPtr + 12, info.fpsDen);
                    mem.putLong(outPtr + 16, info.frameCount);
                    mem.putLong(outPtr + 24, info.durationMs);
                    results[0] = Val.fromI32(0);
                });

        // video_decode_to_gfx(handle: i32, target: i32) -> i64
        // target: 0 = terminal's built-in gfx framebuffer, 1 = attached Screen cluster.
        // Returns pts_ms on success, -1 on EOF, -2 on error.
        addEnvFunc(store, funcs, funcMap, "video_decode_to_gfx",
                new Type[]{Type.I32, Type.I32}, new Type[]{Type.I64},
                (caller, params, results) -> {
                    if (childBridge == null) { results[0] = Val.fromI64(-2); return; }
                    int handle = params[0].i32();
                    int target = params[1].i32();
                    long pts = childBridge.videoDecodeToGfx(handle, target);
                    results[0] = Val.fromI64(pts);
                });

        // video_seek(handle: i32, pts_ms: i64) -> i32
        addEnvFunc(store, funcs, funcMap, "video_seek",
                new Type[]{Type.I32, Type.I64}, new Type[]{Type.I32},
                (caller, params, results) -> {
                    if (childBridge == null) { results[0] = Val.fromI32(-1); return; }
                    int handle = params[0].i32();
                    long pts = params[1].i64();
                    results[0] = Val.fromI32(childBridge.videoSeek(handle, pts));
                });

        // video_close(handle: i32) -> i32
        addEnvFunc(store, funcs, funcMap, "video_close",
                new Type[]{Type.I32}, new Type[]{Type.I32},
                (caller, params, results) -> {
                    if (childBridge == null) { results[0] = Val.fromI32(-1); return; }
                    results[0] = Val.fromI32(childBridge.videoClose(params[0].i32()));
                });

        // gfx_init(target: i32, width: i32, height: i32) -> i32
        // target: 0 = terminal, 1 = screen.
        addEnvFunc(store, funcs, funcMap, "gfx_init",
                new Type[]{Type.I32, Type.I32, Type.I32}, new Type[]{Type.I32},
                (caller, params, results) -> {
                    if (childBridge == null) { results[0] = Val.fromI32(-1); return; }
                    int target = params[0].i32();
                    int w = params[1].i32();
                    int h = params[2].i32();
                    results[0] = Val.fromI32(childBridge.gfxInit(target, w, h));
                });

        // gfx_set_mode(target: i32, mode: i32) -> i32
        addEnvFunc(store, funcs, funcMap, "gfx_set_mode",
                new Type[]{Type.I32, Type.I32}, new Type[]{Type.I32},
                (caller, params, results) -> {
                    if (childBridge == null) { results[0] = Val.fromI32(-1); return; }
                    int target = params[0].i32();
                    int mode = params[1].i32();
                    results[0] = Val.fromI32(childBridge.gfxSetMode(target, mode));
                });

        // screen_query_dims(out_ptr: i32) -> i32
        // Writes two u32 to child memory at out_ptr: [width, height]. Returns
        // 0 on success, -1 if no Screen cluster is currently attached. The
        // player uses this to size its decoder to the cluster's native
        // resolution before calling video_open.
        addEnvFunc(store, funcs, funcMap, "screen_query_dims",
                new Type[]{Type.I32}, new Type[]{Type.I32},
                (caller, params, results) -> {
                    if (childBridge == null) { results[0] = Val.fromI32(-1); return; }
                    long packed = childBridge.screenQueryDims();
                    if (packed < 0) { results[0] = Val.fromI32(-1); return; }
                    int outPtr = params[0].i32();
                    int w = (int) (packed >>> 32);
                    int h = (int) (packed & 0xFFFFFFFFL);
                    ByteBuffer mem = store.data().memory.buffer(store);
                    mem.order(ByteOrder.LITTLE_ENDIAN);
                    mem.putInt(outPtr,     w);
                    mem.putInt(outPtr + 4, h);
                    results[0] = Val.fromI32(0);
                });

        // screen_set_power(on: i32) -> ()
        // Turn the attached Screen cluster on (nonzero) or off (0). Off
        // reverts each member block's face to the "no signal" texture.
        // No-op if no cluster is attached. Signature must match the
        // kernel-side export in ComputerInstance.createHostFunctions
        // because both bindings share the `screen_set_power` symbol
        // in the terminal-os link graph via ecm-host-abi.
        addEnvFunc(store, funcs, funcMap, "screen_set_power",
                new Type[]{Type.I32}, new Type[]{},
                (caller, params, results) -> {
                    if (childBridge == null) return;
                    childBridge.screenSetPower(params[0].i32() != 0);
                });

        // screen_set_pixel_format(format: i32) -> ()
        // Switch the attached screen cluster between indexed8 (0) and
        // rgba8888 (1). The host stages a worker-thread op that writes
        // the format byte into kernel WASM at SCREEN_GFX_BASE+0x10,
        // zeros the pixel region for the new format's byte count, and
        // bumps both dirty counters so clients pick up the layout
        // change. Signature must match the kernel-side export.
        addEnvFunc(store, funcs, funcMap, "screen_set_pixel_format",
                new Type[]{Type.I32}, new Type[]{},
                (caller, params, results) -> {
                    if (childBridge == null) return;
                    childBridge.screenSetPixelFormat(params[0].i32());
                });

        // screen_put_frame_rgba(data_ptr: i32, w: i32, h: i32) -> i32
        // Read w*h*4 bytes of packed RGBA from child memory at data_ptr
        // and stage a full-frame push into the screen cluster's pixel
        // region. Returns 0 on success, -1 on error.
        addEnvFunc(store, funcs, funcMap, "screen_put_frame_rgba",
                new Type[]{Type.I32, Type.I32, Type.I32}, new Type[]{Type.I32},
                (caller, params, results) -> {
                    if (childBridge == null) { results[0] = Val.fromI32(-1); return; }
                    int dataPtr = params[0].i32();
                    int w = params[1].i32();
                    int h = params[2].i32();
                    int byteCount = w * h * 4;
                    if (w <= 0 || h <= 0 || byteCount <= 0) {
                        results[0] = Val.fromI32(-1); return;
                    }
                    byte[] rgba = new byte[byteCount];
                    ByteBuffer mem = store.data().memory.buffer(store);
                    mem.position(dataPtr);
                    mem.get(rgba, 0, byteCount);
                    results[0] = Val.fromI32(childBridge.screenPutFrameRgba(w, h, rgba));
                });

        // poll_oneoff(in_ptr, out_ptr, nsubscriptions, nevents_ptr) -> errno
        // Minimal implementation that handles CLOCK subscriptions for std::thread::sleep.
        // Subscription struct (48 bytes):
        //   userdata u64 @0, tag u8 @8, [pad to 16],
        //   clock: id u32 @16, timeout u64 @24, precision u64 @32, flags u16 @40
        // Event struct (32 bytes):
        //   userdata u64 @0, error u16 @8, type u8 @10, [pad], fd_readwrite (16 bytes) @16
        addFunc(store, funcs, funcMap, "poll_oneoff",
                new Type[]{Type.I32, Type.I32, Type.I32, Type.I32}, new Type[]{Type.I32},
                (caller, params, results) -> {
                    int inPtr = params[0].i32();
                    int outPtr = params[1].i32();
                    int nSubs = params[2].i32();
                    int neventsPtr = params[3].i32();

                    ByteBuffer mem = store.data().memory.buffer(store);
                    mem.order(ByteOrder.LITTLE_ENDIAN);

                    long minTimeoutNanos = Long.MAX_VALUE;
                    long minUserdata = 0;
                    boolean haveClockSub = false;

                    for (int i = 0; i < nSubs; i++) {
                        int subPtr = inPtr + i * 48;
                        long userdata = mem.getLong(subPtr);
                        int tag = mem.get(subPtr + 8) & 0xFF;
                        if (tag == 0) { // CLOCK
                            long timeout = mem.getLong(subPtr + 24);
                            int flags = mem.getShort(subPtr + 40) & 0xFFFF;
                            // flags bit 0 = SUBCLOCKFLAGS_SUBSCRIPTION_CLOCK_ABSTIME
                            long relNanos;
                            if ((flags & 1) != 0) {
                                long nowNanos = System.currentTimeMillis() * 1_000_000L;
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
                        // Non-clock subscriptions are ignored in this minimal impl
                    }

                    if (haveClockSub && minTimeoutNanos > 0) {
                        long ms = minTimeoutNanos / 1_000_000L;
                        if (ms > 0 && childBridge != null) {
                            childBridge.sleepMs((int) Math.min(60_000L, ms));
                        } else if (ms > 0) {
                            try { Thread.sleep(Math.min(60_000L, ms)); }
                            catch (InterruptedException e) { Thread.currentThread().interrupt(); }
                        }
                    }

                    if (haveClockSub) {
                        // Write a single CLOCK event back
                        // Re-fetch the buffer in case sleep advanced state
                        mem = store.data().memory.buffer(store);
                        mem.order(ByteOrder.LITTLE_ENDIAN);
                        // Zero the 32-byte event
                        for (int i = 0; i < 32; i++) mem.put(outPtr + i, (byte) 0);
                        mem.putLong(outPtr, minUserdata);     // userdata
                        mem.putShort(outPtr + 8, (short) 0);  // error
                        mem.put(outPtr + 10, (byte) 0);       // type = CLOCK
                        mem.putInt(neventsPtr, 1);
                    } else {
                        mem.putInt(neventsPtr, 0);
                    }
                    results[0] = Val.fromI32(ERRNO_SUCCESS);
                });
    }

    /**
     * Helper: write a UTF-8 string to a child's WASM memory buffer.
     * Returns the number of bytes written, or -1 on error.
     */
    private static int writeStringToChildMemory(Store<WasiState> store, String s, int bufPtr, int bufLen) {
        if (s == null) return -1;
        try {
            byte[] data = s.getBytes(StandardCharsets.UTF_8);
            int n = Math.min(data.length, bufLen);
            ByteBuffer mem = store.data().memory.buffer(store);
            for (int i = 0; i < n; i++) mem.put(bufPtr + i, data[i]);
            return n;
        } catch (Exception e) {
            return -1;
        }
    }

    /**
     * Resolve a relative path against the child process's storage path,
     * with the same path-traversal sanitization used by path_open.
     * Returns null if the path is invalid or escapes the storage root.
     */
    private static Path resolveChildPath(Store<WasiState> store, String pathStr) {
        if (pathStr == null) return null;
        if (pathStr.contains("..") || pathStr.startsWith("/")) return null;
        Path filePath = store.data().storagePath.resolve(pathStr).normalize();
        if (!filePath.startsWith(store.data().storagePath)) return null;
        return filePath;
    }

    // --- Helper: scatter-gather fd_write ---

    private static int wasifdWrite(Store<WasiState> store, int fd, int iovsPtr, int iovsLen, int nwrittenPtr) {
        WasiFileDescriptor desc = store.data().fdTable.get(fd);
        if (desc == null) {
            EvansComputerMod.LOGGER.debug("WASI fd_write: fd={} NOT FOUND in fdTable", fd);
            return ERRNO_BADF;
        }

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
                if (written < 0) {
                    EvansComputerMod.LOGGER.debug("WASI fd_write: fd={} write returned {}", fd, written);
                    return ERRNO_BADF;
                }
                total += written;
            } catch (IOException e) {
                EvansComputerMod.LOGGER.error("WASI fd_write: fd={} IOException", fd, e);
                return ERRNO_BADF;
            }
        }

        // Log writes to non-stdio FDs (file writes)
        if (fd > 2) {
            EvansComputerMod.LOGGER.debug("WASI fd_write: fd={} wrote {} bytes (desc={})", fd, total, desc.getClass().getSimpleName());
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
