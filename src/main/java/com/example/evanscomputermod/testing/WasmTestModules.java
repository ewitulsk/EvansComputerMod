package com.example.evanscomputermod.testing;

import java.io.ByteArrayOutputStream;
import java.nio.charset.StandardCharsets;

/**
 * Tiny hand-assembled WASI programs for host tests (no Rust build needed).
 */
public final class WasmTestModules {

    private WasmTestModules() {}

    /** Byte offset of the spin counter in {@link #spinCounter}'s memory (an i32). */
    public static final int SPIN_COUNTER_ADDR = 32;

    /**
     * A WASI program that never exits. Its loop increments the i32 at
     * {@link #SPIN_COUNTER_ADDR} {@code innerIterations} times in pure wasm
     * (no host calls), then writes one {@code '.'} to stdout with
     * {@code fd_write}, forever. So the number of dots on stdout counts host
     * calls, and the memory counter counts guest loop iterations.
     *
     * <pre>
     * (func $_start (local $j i32)
     *   (i32.store (i32.const 0) (i32.const 16))   ;; iovec.buf
     *   (i32.store (i32.const 4) (i32.const 1))    ;; iovec.len
     *   (i32.store8 (i32.const 16) (i32.const 46)) ;; '.'
     *   (loop $outer
     *     (local.set $j (i32.const 0))
     *     (loop $inner
     *       (i32.store (i32.const 32) (i32.add (i32.load (i32.const 32)) (i32.const 1)))
     *       (br_if $inner (i32.lt_u (local.tee $j (i32.add (local.get $j) (i32.const 1)))
     *                               (i32.const innerIterations))))
     *     (drop (call $fd_write (i32.const 1) (i32.const 0) (i32.const 1) (i32.const 8)))
     *     (br $outer)))
     * </pre>
     */
    public static byte[] spinCounter(int innerIterations) {
        ByteArrayOutputStream out = new ByteArrayOutputStream();
        out.writeBytes(new byte[]{0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00});

        // Types: 0 = (i32 i32 i32 i32) -> i32 (fd_write), 1 = () -> ()
        section(out, 1, bytes(0x02,
                0x60, 0x04, 0x7F, 0x7F, 0x7F, 0x7F, 0x01, 0x7F,
                0x60, 0x00, 0x00));

        // Import: wasi_snapshot_preview1.fd_write as func 0
        ByteArrayOutputStream imp = new ByteArrayOutputStream();
        imp.write(0x01);
        name(imp, "wasi_snapshot_preview1");
        name(imp, "fd_write");
        imp.write(0x00); // func
        imp.write(0x00); // type 0
        section(out, 2, imp.toByteArray());

        section(out, 3, bytes(0x01, 0x01));            // func 1 : type 1
        section(out, 5, bytes(0x01, 0x00, 0x01));      // one memory, min 1 page

        ByteArrayOutputStream exp = new ByteArrayOutputStream();
        exp.write(0x02);
        name(exp, "memory");
        exp.write(0x02);
        exp.write(0x00);
        name(exp, "_start");
        exp.write(0x00);
        exp.write(0x01);
        section(out, 7, exp.toByteArray());

        ByteArrayOutputStream body = new ByteArrayOutputStream();
        body.writeBytes(bytes(0x01, 0x01, 0x7F));       // one i32 local
        body.writeBytes(bytes(0x41, 0x00, 0x41, 0x10, 0x36, 0x02, 0x00));
        body.writeBytes(bytes(0x41, 0x04, 0x41, 0x01, 0x36, 0x02, 0x00));
        body.writeBytes(bytes(0x41, 0x10, 0x41, 0x2E, 0x3A, 0x00, 0x00));
        body.writeBytes(bytes(0x03, 0x40));             // loop $outer
        body.writeBytes(bytes(0x41, 0x00, 0x21, 0x00)); //   j = 0
        body.writeBytes(bytes(0x03, 0x40));             //   loop $inner
        body.writeBytes(bytes(0x41, SPIN_COUNTER_ADDR, 0x41, SPIN_COUNTER_ADDR, 0x28, 0x02, 0x00,
                0x41, 0x01, 0x6A, 0x36, 0x02, 0x00));   //     counter += 1
        body.writeBytes(bytes(0x20, 0x00, 0x41, 0x01, 0x6A, 0x22, 0x00)); // j = j + 1 (tee)
        body.write(0x41);
        sleb(body, innerIterations);
        body.writeBytes(bytes(0x49, 0x0D, 0x00));       //     br_if $inner (j < n)
        body.write(0x0B);                               //   end
        body.writeBytes(bytes(0x41, 0x01, 0x41, 0x00, 0x41, 0x01, 0x41, 0x08, 0x10, 0x00, 0x1A));
        body.writeBytes(bytes(0x0C, 0x00));             //   br $outer
        body.write(0x0B);                               // end loop
        body.write(0x0B);                               // end func

        ByteArrayOutputStream code = new ByteArrayOutputStream();
        code.write(0x01);
        uleb(code, body.size());
        code.writeBytes(body.toByteArray());
        section(out, 10, code.toByteArray());
        return out.toByteArray();
    }

    private static void section(ByteArrayOutputStream out, int id, byte[] content) {
        out.write(id);
        uleb(out, content.length);
        out.writeBytes(content);
    }

    private static void name(ByteArrayOutputStream out, String s) {
        byte[] b = s.getBytes(StandardCharsets.UTF_8);
        uleb(out, b.length);
        out.writeBytes(b);
    }

    private static byte[] bytes(int... v) {
        byte[] b = new byte[v.length];
        for (int i = 0; i < v.length; i++) b[i] = (byte) v[i];
        return b;
    }

    private static void uleb(ByteArrayOutputStream out, long v) {
        do {
            int b = (int) (v & 0x7F);
            v >>>= 7;
            out.write(v != 0 ? b | 0x80 : b);
        } while (v != 0);
    }

    private static void sleb(ByteArrayOutputStream out, long v) {
        while (true) {
            int b = (int) (v & 0x7F);
            v >>= 7;
            boolean done = (v == 0 && (b & 0x40) == 0) || (v == -1 && (b & 0x40) != 0);
            out.write(done ? b : b | 0x80);
            if (done) return;
        }
    }
}
