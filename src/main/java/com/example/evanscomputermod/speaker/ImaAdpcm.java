package com.example.evanscomputermod.speaker;

/**
 * IMA ADPCM, 4 bits per sample (4:1 against 16-bit PCM), mono. Used to send
 * speaker audio to clients.
 *
 * <p>A block is self-contained, so a lost or reordered packet only costs its
 * own samples: {@code [i16 LE first sample][u8 step index][u8 0][nibbles, low
 * nibble first]}. The first sample is stored verbatim; the remaining
 * {@code n - 1} samples take one nibble each.
 */
public final class ImaAdpcm {

    private static final int[] INDEX_TABLE = {-1, -1, -1, -1, 2, 4, 6, 8, -1, -1, -1, -1, 2, 4, 6, 8};

    private static final int[] STEP_TABLE = {
            7, 8, 9, 10, 11, 12, 13, 14, 16, 17, 19, 21, 23, 25, 28, 31, 34, 37, 41, 45,
            50, 55, 60, 66, 73, 80, 88, 97, 107, 118, 130, 143, 157, 173, 190, 209, 230,
            253, 279, 307, 337, 371, 408, 449, 494, 544, 598, 658, 724, 796, 876, 963,
            1060, 1166, 1282, 1411, 1552, 1707, 1878, 2066, 2272, 2499, 2749, 3024, 3327,
            3660, 4026, 4428, 4871, 5358, 5894, 6484, 7132, 7845, 8630, 9493, 10442, 11487,
            12635, 13899, 15289, 16818, 18500, 20350, 22385, 24623, 27086, 29794, 32767
    };

    public static final int HEADER_BYTES = 4;

    /** Encoder state carried between blocks (keeps the step size adapted). */
    private int index;

    /** Encoded size of a block of {@code samples} samples. */
    public static int encodedSize(int samples) {
        return samples <= 0 ? 0 : HEADER_BYTES + samples / 2;
    }

    /** Encode {@code count} samples starting at {@code off} into one block. */
    public byte[] encode(short[] pcm, int off, int count) {
        if (count <= 0) return new byte[0];
        byte[] out = new byte[encodedSize(count)];
        int predictor = pcm[off];
        out[0] = (byte) predictor;
        out[1] = (byte) (predictor >> 8);
        out[2] = (byte) index;
        int step = STEP_TABLE[index];
        for (int i = 1; i < count; i++) {
            int diff = pcm[off + i] - predictor;
            int nibble = 0;
            if (diff < 0) {
                nibble = 8;
                diff = -diff;
            }
            int delta = step >> 3;
            if (diff >= step) {
                nibble |= 4;
                diff -= step;
                delta += step;
            }
            if (diff >= step >> 1) {
                nibble |= 2;
                diff -= step >> 1;
                delta += step >> 1;
            }
            if (diff >= step >> 2) {
                nibble |= 1;
                delta += step >> 2;
            }
            predictor = clamp16((nibble & 8) != 0 ? predictor - delta : predictor + delta);
            index = clampIndex(index + INDEX_TABLE[nibble]);
            step = STEP_TABLE[index];
            int pos = HEADER_BYTES + (i - 1) / 2;
            if (((i - 1) & 1) == 0) out[pos] = (byte) nibble;
            else out[pos] |= (byte) (nibble << 4);
        }
        return out;
    }

    /** Decode a block of {@code count} samples. Returns null if the block is malformed. */
    public static short[] decode(byte[] block, int count) {
        if (count <= 0 || count > 65536 || block == null || block.length < encodedSize(count)) return null;
        short[] out = new short[count];
        int predictor = (short) ((block[0] & 0xFF) | (block[1] << 8));
        int index = block[2] & 0xFF;
        if (index > 88) return null;
        out[0] = (short) predictor;
        int step = STEP_TABLE[index];
        for (int i = 1; i < count; i++) {
            int b = block[HEADER_BYTES + (i - 1) / 2] & 0xFF;
            int nibble = ((i - 1) & 1) == 0 ? b & 0x0F : b >> 4;
            int delta = step >> 3;
            if ((nibble & 4) != 0) delta += step;
            if ((nibble & 2) != 0) delta += step >> 1;
            if ((nibble & 1) != 0) delta += step >> 2;
            predictor = clamp16((nibble & 8) != 0 ? predictor - delta : predictor + delta);
            index = clampIndex(index + INDEX_TABLE[nibble]);
            step = STEP_TABLE[index];
            out[i] = (short) predictor;
        }
        return out;
    }

    private static int clamp16(int v) {
        return Math.max(-32768, Math.min(32767, v));
    }

    private static int clampIndex(int i) {
        return Math.max(0, Math.min(88, i));
    }
}
