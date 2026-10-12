package com.example.evanscomputermod.computer.ui;

import com.example.evanscomputermod.computer.ui.proto.UiProto;

/** A protocol error: the compositor sends {@code Error{code, message}} and closes the connection (§3). */
public final class UiError extends Exception {
    public final UiProto.ErrorCode code;

    public UiError(UiProto.ErrorCode code, String message) {
        super(message, null, false, false);
        this.code = code;
    }

    public static UiError malformed(String m) {
        return new UiError(UiProto.ErrorCode.MALFORMED, m);
    }

    public static UiError limit(String m) {
        return new UiError(UiProto.ErrorCode.LIMIT, m);
    }

    public static UiError unknown(String m) {
        return new UiError(UiProto.ErrorCode.UNKNOWN_OBJECT, m);
    }

    public static UiError badState(String m) {
        return new UiError(UiProto.ErrorCode.BAD_STATE, m);
    }

    public static UiError notPermitted(String m) {
        return new UiError(UiProto.ErrorCode.NOT_PERMITTED, m);
    }
}
