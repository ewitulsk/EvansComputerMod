package com.example.evanscomputermod.computer.video;

import com.example.evanscomputermod.computer.TerminalDisplay;
import org.junit.jupiter.api.Test;

import java.util.concurrent.CountDownLatch;
import java.util.concurrent.atomic.AtomicInteger;
import java.util.concurrent.atomic.AtomicReference;

import static org.junit.jupiter.api.Assertions.*;

/**
 * Concurrent stress: one writer pushes gfx blobs into a
 * {@link TerminalDisplay} while two readers repeatedly call
 * {@code gfxToBytes}. After the stress ends, every read must have
 * produced a structurally consistent blob (sizes matching width*height
 * and the magic number intact). Without {@code synchronized} on both
 * the writer and reader, readers would see half-updated pixel buffers
 * whose length disagreed with gfxWidth*gfxHeight.
 */
class TerminalDisplayGfxThreadSafetyTest {

    @Test
    void concurrent_writer_and_readers_never_see_torn_state() throws Exception {
        TerminalDisplay td = new TerminalDisplay();
        byte[] palette = Rgb332Palette.bytes();

        // Two frame sizes so the writer alternates pixel-buffer reallocation,
        // which is exactly where a torn read would happen.
        byte[] pixelsA = new byte[320 * 200];
        byte[] pixelsB = new byte[160 * 90];
        for (int i = 0; i < pixelsA.length; i++) pixelsA[i] = (byte) (i & 0xFF);
        for (int i = 0; i < pixelsB.length; i++) pixelsB[i] = (byte) (~i & 0xFF);

        byte[] blobA = GfxFrameBlob.build(320, 200, 1, palette, pixelsA, 1, 1);
        byte[] blobB = GfxFrameBlob.build(160, 90,  1, palette, pixelsB, 2, 2);

        final int ITERATIONS = 2_000;
        CountDownLatch ready = new CountDownLatch(3);
        CountDownLatch go    = new CountDownLatch(1);
        AtomicInteger torn   = new AtomicInteger();
        AtomicReference<String> tornReason = new AtomicReference<>();

        Thread writer = new Thread(() -> {
            ready.countDown();
            try { go.await(); } catch (InterruptedException ignored) { return; }
            for (int i = 0; i < ITERATIONS; i++) {
                td.setGfxFromBytes((i & 1) == 0 ? blobA : blobB);
            }
        }, "gfx-writer");

        Runnable readerJob = () -> {
            ready.countDown();
            try { go.await(); } catch (InterruptedException ignored) { return; }
            for (int i = 0; i < ITERATIONS; i++) {
                byte[] serialized = td.gfxToBytes();
                if (serialized == null) continue;
                int magic = (serialized[0] & 0xFF) | ((serialized[1] & 0xFF) << 8);
                if (magic != TerminalDisplay.GFX_MAGIC) {
                    torn.incrementAndGet();
                    tornReason.compareAndSet(null, "bad magic 0x" + Integer.toHexString(magic));
                    return;
                }
                int w = (serialized[4] & 0xFF) | ((serialized[5] & 0xFF) << 8);
                int h = (serialized[6] & 0xFF) | ((serialized[7] & 0xFF) << 8);
                int expected = 0x400 + w * h;
                if (serialized.length != expected) {
                    torn.incrementAndGet();
                    tornReason.compareAndSet(null,
                            "len=" + serialized.length + " expected=" + expected
                            + " (w=" + w + " h=" + h + ")");
                    return;
                }
            }
        };
        Thread reader1 = new Thread(readerJob, "gfx-reader-1");
        Thread reader2 = new Thread(readerJob, "gfx-reader-2");

        writer.start();
        reader1.start();
        reader2.start();
        ready.await();
        go.countDown();
        writer.join();
        reader1.join();
        reader2.join();

        assertEquals(0, torn.get(),
                "observed torn read: " + tornReason.get());
    }
}
