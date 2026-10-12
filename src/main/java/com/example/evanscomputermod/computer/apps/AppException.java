package com.example.evanscomputermod.computer.apps;

/** An app-manager request that can't be done; {@link #code} is the {@code app_ctl} status. */
public final class AppException extends Exception {
    public static final int NOT_FOUND = -2;
    public static final int DENIED = -13;
    public static final int INVALID = -22;
    /** Several apps open this file type and none is the default: show a chooser. */
    public static final int CHOOSE = -35;

    public final int code;

    public AppException(int code, String message) {
        super(message);
        this.code = code;
    }
}
