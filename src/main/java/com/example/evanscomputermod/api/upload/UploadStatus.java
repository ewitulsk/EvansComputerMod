package com.example.evanscomputermod.api.upload;

/**
 * Upload states and errors: what {@code upload_poll} returns to a program
 * (abi/child-abi.toml {@code [imports.upload]}) and what the sidecar sends
 * the player's game.
 */
public final class UploadStatus {
    /** Accepted (sink calls) / waiting for the player to pick a file (poll). */
    public static final int OK = 0;
    public static final int PICKING = 0;
    /** The file is on its way. */
    public static final int RECEIVING = 1;
    /** The file is on the computer. */
    public static final int DONE = 2;

    /** No such upload. */
    public static final int UNKNOWN = -1;
    /** The player closed the file picker, or the program stopped waiting. */
    public static final int CANCELLED = -10;
    /** The file is larger than the server takes. */
    public static final int TOO_LARGE = -11;
    /** The computer's storage would go over the server's limit. */
    public static final int STORAGE_FULL = -12;
    /** The server doesn't let this player upload. */
    public static final int NOT_PERMITTED = -13;
    /** The transfer broke: the player left or moved away, the file couldn't be read, or bytes went missing. */
    public static final int FAILED = -14;
    /** Nobody with the add-on is using the computer. */
    public static final int NO_CLIENT = -15;

    private UploadStatus() {}

    /** A short reason for a player, or null for a state that isn't an error. */
    public static String describe(int status) {
        return switch (status) {
            case CANCELLED -> "cancelled";
            case TOO_LARGE -> "the file is larger than this server allows";
            case STORAGE_FULL -> "the computer's storage is full";
            case NOT_PERMITTED -> "this server doesn't let you upload files";
            case FAILED -> "the transfer failed";
            case NO_CLIENT -> "your game doesn't have the upload add-on";
            case UNKNOWN -> "no such upload";
            default -> null;
        };
    }
}
