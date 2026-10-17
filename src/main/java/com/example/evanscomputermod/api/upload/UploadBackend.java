package com.example.evanscomputermod.api.upload;

import net.minecraft.server.level.ServerPlayer;

/**
 * Copies a file from a player's real computer into a computer in the game:
 * the {@code evanscomputermod-upload} sidecar implements it (the file picker
 * runs in the player's game, the bytes come to the server over the network).
 * Without it, computers have no upload port ({@code upload_supported} is 0).
 *
 * <p>The main mod owns the computer side ({@link UploadSink}: where the file
 * goes, limits, the program that asked); the backend owns the player side.
 * Every method runs on the server thread.
 */
public interface UploadBackend {
    /** Bumped when this interface changes incompatibly; a backend with another version is ignored. */
    int CURRENT_API_VERSION = 1;

    int apiVersion();

    String name();

    /** Whether this player's game has the add-on, so it can show a file picker and send the file. */
    boolean canUpload(ServerPlayer player);

    /** Whether the server lets this player upload (its config: on or off, operators only). */
    boolean permitted(ServerPlayer player);

    /** The largest file the server takes, in bytes. */
    long maxFileBytes();

    /** The most a computer's files may total after an upload, in bytes; 0 for no limit. */
    long maxComputerBytes();

    /**
     * Ask {@code player}'s game to let them pick a file and send it to
     * {@code sink}. The backend reports what happens through the sink
     * ({@link UploadSink#begin}, {@link UploadSink#write}, {@link UploadSink#finish},
     * {@link UploadSink#fail}).
     */
    void start(ServerPlayer player, UploadSink sink);

    /** The computer gave up on {@code sink} (its program exited, or the computer stopped). */
    void cancel(UploadSink sink);

    /**
     * Scenarios: {@code player} is a scenario's stand-in player, which has no
     * game to show a file picker in. Its next upload "picks" {@code file}
     * (null: none, so it has no add-on). False when the backend can't do that.
     * (docs/SCENARIO_EXCEPTIONS.md, "upload file picker".)
     */
    default boolean simulatePick(ServerPlayer player, java.nio.file.Path file) {
        return false;
    }
}
