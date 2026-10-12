package com.example.evanscomputermod.computer.ui.proto;

/** A buffer that is not (yet) a valid UI protocol message. */
public final class UiProtoException extends Exception {
    public enum Kind {
        /** Not a whole message yet; {@link #needed()} bytes are needed in total. */
        INCOMPLETE,
        UNKNOWN_OPCODE,
        TOO_LARGE,
        MALFORMED
    }

    private final Kind kind;
    private final int needed;

    private UiProtoException(Kind kind, String message, int needed) {
        super(message, null, false, false);
        this.kind = kind;
        this.needed = needed;
    }

    public static UiProtoException malformed(String message) {
        return new UiProtoException(Kind.MALFORMED, message, 0);
    }

    public static UiProtoException unknownOpcode(int opcode) {
        return new UiProtoException(Kind.UNKNOWN_OPCODE, String.format("unknown opcode 0x%04x", opcode), 0);
    }

    public static UiProtoException tooLarge(long length) {
        return new UiProtoException(Kind.TOO_LARGE, "message body of " + length + " bytes is too large", 0);
    }

    public static UiProtoException incomplete(int needed) {
        return new UiProtoException(Kind.INCOMPLETE, "incomplete message (need " + needed + " bytes)", needed);
    }

    public Kind kind() {
        return kind;
    }

    public int needed() {
        return needed;
    }

    /** The protocol error code the compositor closes the connection with. */
    public UiProto.ErrorCode errorCode() {
        return switch (kind) {
            case UNKNOWN_OPCODE -> UiProto.ErrorCode.UNKNOWN_OPCODE;
            case TOO_LARGE -> UiProto.ErrorCode.LIMIT;
            default -> UiProto.ErrorCode.MALFORMED;
        };
    }
}
