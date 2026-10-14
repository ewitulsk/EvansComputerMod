package com.example.evanscomputermod.computer.ui.proto;

import java.util.ArrayList;
import java.util.List;

/**
 * Framing for UI protocol messages: {@code u16 opcode · u16 flags (0) · u32 length · body}, little-endian
 * (docs/gui/UI_PROTOCOL.md §3). Decoding is strict; see abi/ui-protocol.toml.
 */
public final class UiCodec {
    public static final int HEADER_LEN = 8;
    /** Node id of a tree surface's implicit root group. */
    public static final int ROOT_NODE = 0;
    /** TreeOp index that appends after the parent's last child. */
    public static final int APPEND = 0xFFFFFFFF;

    private UiCodec() {}

    /** One decoded message and the bytes it used. */
    public record Decoded(UiProto.Message message, int length) {}

    /** Append {@code m} (header and body) to {@code w}. */
    public static void encode(UiProto.Message m, UiWriter w) {
        int start = w.length();
        w.u16(m.opcode());
        w.u16(0);
        w.u32(0);
        m.encBody(w);
        w.patchU32(start + 4, w.length() - start - HEADER_LEN);
    }

    public static byte[] encode(UiProto.Message m) {
        UiWriter w = new UiWriter();
        encode(m, w);
        return w.toByteArray();
    }

    public static byte[] encodeAll(List<? extends UiProto.Message> ms) {
        UiWriter w = new UiWriter();
        for (UiProto.Message m : ms) encode(m, w);
        return w.toByteArray();
    }

    /**
     * Decode the first message in {@code buf[off, off + len)}. Throws {@link UiProtoException.Kind#INCOMPLETE}
     * when more bytes are needed (only while the header is valid so far).
     */
    public static Decoded decode(byte[] buf, int off, int len) throws UiProtoException {
        if (len < HEADER_LEN) throw UiProtoException.incomplete(HEADER_LEN);
        int opcode = (buf[off] & 0xFF) | (buf[off + 1] & 0xFF) << 8;
        int flags = (buf[off + 2] & 0xFF) | (buf[off + 3] & 0xFF) << 8;
        long bodyLen = ((buf[off + 4] & 0xFF) | (buf[off + 5] & 0xFF) << 8 | (buf[off + 6] & 0xFF) << 16
                | (long) (buf[off + 7] & 0xFF) << 24);
        if (flags != 0) throw UiProtoException.malformed("header flags must be 0");
        if (!UiProto.isKnown(opcode)) throw UiProtoException.unknownOpcode(opcode);
        if (bodyLen > UiProto.MAX_MESSAGE) throw UiProtoException.tooLarge(bodyLen);
        int total = HEADER_LEN + (int) bodyLen;
        if (len < total) throw UiProtoException.incomplete(total);
        UiReader r = new UiReader(buf, off + HEADER_LEN, (int) bodyLen);
        UiProto.Message m = UiProto.decBody(opcode, r);
        r.finish();
        return new Decoded(m, total);
    }

    /** Decode a buffer that holds exactly one whole message. */
    public static UiProto.Message decodeExact(byte[] buf) throws UiProtoException {
        Decoded d = decode(buf, 0, buf.length);
        if (d.length() != buf.length) throw UiProtoException.malformed("trailing bytes after the message");
        return d.message();
    }

    /** Decode a buffer of whole messages (one {@code ui_send}). */
    public static List<UiProto.Message> decodeAll(byte[] buf, int off, int len) throws UiProtoException {
        List<UiProto.Message> out = new ArrayList<>();
        int end = off + len;
        while (off < end) {
            Decoded d = decode(buf, off, end - off);
            out.add(d.message());
            off += d.length();
        }
        return out;
    }

    /** Canonical JSON of a message (the shared test-vector format). */
    public static String toJson(UiProto.Message m) {
        StringBuilder o = new StringBuilder();
        m.json(o);
        return o.toString();
    }
}
