package com.example.evanscomputermod.card.nfc;

import com.example.evanscomputermod.radio.api.Band;
import com.example.evanscomputermod.radio.api.Channel;
import org.jetbrains.annotations.Nullable;

import java.util.Arrays;

/**
 * The near-field air interface (docs/cards/NFC_MIX.md, "NFC air frames"): every frame is
 * {@code cmd(1) len(1) body(len)} at modulation {@link #MODULATION} on {@link #CHANNEL}.
 * Readers send commands; energised cards answer. Pure: encoding, decoding and timing only.
 */
public final class NfcAir {
    /** 13.56 MHz; the 848 kHz subcarrier sidebands set the bandwidth (so a 106 kb/s frame has processing gain). */
    public static final Channel CHANNEL = new Channel(13.56e6, 848e3);
    public static final String MODULATION = "NF-106";
    public static final double BIT_RATE = 106_000;
    /** Anticollision: POLL replies go out in one of these slots (UID[7] & 15), this far apart (longer than a POLL reply's 830 us). */
    public static final int SLOTS = 16;
    public static final long SLOT_MICROS = 1_000;
    /** The reader hears replies this long after a command (the slots plus the longest frame). */
    public static final long REPLY_WINDOW_MICROS = SLOTS * SLOT_MICROS + 10_000;

    public static final int POLL = 0x01, SELECT = 0x02, AUTH = 0x03, AUTH2 = 0x04, READ = 0x05, WRITE = 0x06, SETKEY = 0x07;
    public static final int ERROR = 0xFF;
    public static final int REPLY = 0x80;
    public static final int ERR_BAD_AUTH = 1, ERR_NO_SESSION = 2, ERR_BAD_SECTOR = 3, ERR_NOT_SELECTED = 4, ERR_MALFORMED = 5;

    /** Kinds bitmask in a POLL reply. */
    public static final int KIND_BANK = 1, KIND_NFC = 2, KIND_SIM = 4;

    public record Frame(int cmd, byte[] body) {
        public byte[] encode() {
            byte[] out = new byte[2 + body.length];
            out[0] = (byte) cmd;
            out[1] = (byte) body.length;
            System.arraycopy(body, 0, out, 2, body.length);
            return out;
        }

        public boolean isError() {
            return cmd == ERROR;
        }

        public int errorReason() {
            return isError() && body.length > 0 ? body[0] & 0xff : 0;
        }

        public byte[] slice(int from, int len) {
            return Arrays.copyOfRange(body, from, from + len);
        }

        @Override
        public String toString() {
            return String.format("Frame[%02x, %d bytes]", cmd, body.length);
        }
    }

    private NfcAir() {}

    public static Frame frame(int cmd, byte[]... parts) {
        int n = 0;
        for (byte[] p : parts) n += p.length;
        byte[] body = new byte[n];
        int o = 0;
        for (byte[] p : parts) {
            System.arraycopy(p, 0, body, o, p.length);
            o += p.length;
        }
        return new Frame(cmd, body);
    }

    public static Frame error(int reason) {
        return new Frame(ERROR, new byte[] {(byte) reason});
    }

    /** Decode a frame, or null if malformed. */
    public static @Nullable Frame decode(byte[] bytes) {
        if (bytes == null || bytes.length < 2) return null;
        int len = bytes[1] & 0xff;
        if (bytes.length != 2 + len) return null;
        return new Frame(bytes[0] & 0xff, Arrays.copyOfRange(bytes, 2, 2 + len));
    }

    /** Airtime of a frame of {@code bytes} bytes, microseconds. */
    public static long durationMicros(int bytes) {
        return Math.max(1, Math.round(bytes * 8 / BIT_RATE * 1e6));
    }

    /** Which anticollision slot a card with this UID answers POLL in. */
    public static int slotOf(long uid) {
        return (int) (uid & (SLOTS - 1));
    }

    public static boolean onBand(double hz) {
        return Band.of(hz) == Band.NEAR_FIELD;
    }
}
