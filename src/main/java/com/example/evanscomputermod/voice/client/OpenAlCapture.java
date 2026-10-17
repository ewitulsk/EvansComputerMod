package com.example.evanscomputermod.voice.client;

//? if <=1.21.1 {
import com.example.evanscomputermod.EvansComputerMod;
import org.lwjgl.openal.AL10;
import org.lwjgl.openal.ALC10;
import org.lwjgl.openal.ALC11;
import org.lwjgl.system.MemoryUtil;

import java.nio.ByteBuffer;
import java.nio.ByteOrder;

/**
 * Our own microphone capture, used when no voice chat mod is installed:
 * OpenAL's capture device (the OpenAL Minecraft already ships, so no extra
 * native code), 48 kHz mono 16-bit, polled on its own thread. Opened only
 * while the player's microphone input is on and something wants their voice;
 * closed otherwise, so the system's "microphone in use" light means it.
 */
final class OpenAlCapture {

    static final int RATE = 48_000;

    private volatile boolean running;
    private Thread thread;
    private long device;
    private String openedName;
    private boolean warned;

    boolean running() {
        return running;
    }

    /** Open the named capture device (empty: the default) and start polling. False if it can't be opened. */
    synchronized boolean start(String name) {
        if (running) return true;
        long dev = ALC11.alcCaptureOpenDevice(name == null || name.isEmpty() ? null : name, RATE, AL10.AL_FORMAT_MONO16, RATE / 2);
        if (dev == 0) {
            if (!warned) EvansComputerMod.LOGGER.warn("Microphone: can't open capture device '{}'", name == null ? "" : name);
            warned = true;
            return false;
        }
        warned = false;
        ALC11.alcCaptureStart(dev);
        device = dev;
        openedName = name;
        running = true;
        thread = new Thread(this::poll, "ECM microphone");
        thread.setDaemon(true);
        thread.start();
        return true;
    }

    String openedName() {
        return openedName;
    }

    synchronized void stop() {
        if (!running) return;
        running = false;
        Thread t = thread;
        if (t != null) {
            t.interrupt();
            try {
                t.join(500);
            } catch (InterruptedException e) {
                Thread.currentThread().interrupt();
            }
        }
        ALC11.alcCaptureStop(device);
        ALC11.alcCaptureCloseDevice(device);
        device = 0;
        thread = null;
    }

    private void poll() {
        ByteBuffer buf = MemoryUtil.memAlloc(RATE);   // half a second of 16-bit mono
        try {
            while (running) {
                int avail = ALC10.alcGetInteger(device, ALC11.ALC_CAPTURE_SAMPLES);
                if (avail >= RATE / 100) {   // at least 10 ms
                    int n = Math.min(avail, RATE / 2);
                    buf.clear();
                    ALC11.alcCaptureSamples(device, buf, n);
                    short[] pcm = new short[n];
                    buf.order(ByteOrder.LITTLE_ENDIAN).asShortBuffer().get(pcm, 0, n);
                    CaptureBuffer.offer(pcm, RATE, CaptureBuffer.Source.OWN);
                }
                try {
                    Thread.sleep(10);
                } catch (InterruptedException e) {
                    break;
                }
            }
        } finally {
            MemoryUtil.memFree(buf);
        }
    }
}
//?}
