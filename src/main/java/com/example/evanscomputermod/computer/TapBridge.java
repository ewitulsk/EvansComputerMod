package com.example.evanscomputermod.computer;

import com.example.evanscomputermod.EvansComputerMod;

import java.io.*;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;

/**
 * TAP bridge for real internet access from Minecraft computers.
 *
 * Opens a Linux TAP device via a helper process and bridges ethernet frames
 * between the NetworkHub and the real network.
 *
 * This uses a helper process approach instead of JNI/JNA for portability:
 * a small shell script opens the TAP device and reads/writes raw frames
 * via stdin/stdout using a simple length-prefixed protocol.
 *
 * Requires: Linux with a pre-configured TAP device (see scripts/setup-tap.sh).
 *
 * Frame protocol on stdin/stdout:
 *   [4 bytes LE length] [frame bytes...]
 */
public class TapBridge implements AutoCloseable {
    private final String deviceName;
    private Process helperProcess;
    private Thread readerThread;
    private Thread writerThread;
    private volatile boolean running = true;
    private final NetworkHub hub;

    // Use a direct file approach instead of a helper process
    private RandomAccessFile tapFile;
    private FileInputStream tapInput;
    private FileOutputStream tapOutput;

    /**
     * Open a TAP bridge using the native /dev/net/tun interface.
     * This requires the TAP device to already be created and configured
     * (via scripts/setup-tap.sh).
     *
     * Falls back to reading/writing the pre-opened TAP device file.
     */
    public TapBridge(String deviceName, NetworkHub hub) throws IOException {
        this.deviceName = deviceName;
        this.hub = hub;

        // Try to open via a helper script that does the ioctl
        // and passes the fd to us via a subprocess
        startHelperProcess();
    }

    /**
     * Start a helper process that opens the TAP device and proxies frames.
     * The helper uses a simple length-prefixed binary protocol on stdin/stdout.
     */
    private void startHelperProcess() throws IOException {
        // The helper script:
        // 1. Opens /dev/net/tun with IFF_TAP | IFF_NO_PI
        // 2. Reads frames from the device, writes length-prefixed to stdout
        // 3. Reads length-prefixed frames from stdin, writes to the device
        //
        // We use Python as the helper since it's widely available and can
        // do the ioctl directly.
        String helperScript = String.join("\n",
            "import sys, os, struct, fcntl, select",
            "TUNSETIFF = 0x400454ca",
            "IFF_TAP = 0x0002",
            "IFF_NO_PI = 0x1000",
            "fd = os.open('/dev/net/tun', os.O_RDWR)",
            "ifr = struct.pack('16sH22s', b'" + deviceName + "', IFF_TAP | IFF_NO_PI, b'\\x00' * 22)",
            "fcntl.ioctl(fd, TUNSETIFF, ifr)",
            "# Set non-blocking for reads",
            "flags = fcntl.fcntl(fd, fcntl.F_GETFL)",
            "fcntl.fcntl(fd, fcntl.F_SETFL, flags | os.O_NONBLOCK)",
            "sys.stdout.buffer.write(b'OK')",
            "sys.stdout.buffer.flush()",
            "import threading",
            "def reader():",
            "    while True:",
            "        try:",
            "            r, _, _ = select.select([fd], [], [], 0.01)",
            "            if r:",
            "                data = os.read(fd, 1514)",
            "                if data:",
            "                    sys.stdout.buffer.write(struct.pack('<I', len(data)))",
            "                    sys.stdout.buffer.write(data)",
            "                    sys.stdout.buffer.flush()",
            "        except:",
            "            break",
            "t = threading.Thread(target=reader, daemon=True)",
            "t.start()",
            "while True:",
            "    hdr = sys.stdin.buffer.read(4)",
            "    if not hdr or len(hdr) < 4:",
            "        break",
            "    length = struct.unpack('<I', hdr)[0]",
            "    if length > 1514:",
            "        break",
            "    frame = sys.stdin.buffer.read(length)",
            "    if len(frame) < length:",
            "        break",
            "    os.write(fd, frame)",
            "os.close(fd)"
        );

        ProcessBuilder pb = new ProcessBuilder("python3", "-c", helperScript);
        pb.redirectErrorStream(false);
        helperProcess = pb.start();

        // Wait for "OK" from helper
        byte[] ok = new byte[2];
        int read = helperProcess.getInputStream().read(ok);
        if (read != 2 || ok[0] != 'O' || ok[1] != 'K') {
            helperProcess.destroyForcibly();
            throw new IOException("TAP helper failed to initialize");
        }

        EvansComputerMod.LOGGER.info("TAP bridge opened: {}", deviceName);

        // Reader thread: reads frames from helper stdout, injects into hub
        readerThread = new Thread(() -> {
            try {
                InputStream in = helperProcess.getInputStream();
                byte[] lenBuf = new byte[4];
                byte[] frameBuf = new byte[1514];

                while (running && helperProcess.isAlive()) {
                    // Read length prefix
                    int bytesRead = 0;
                    while (bytesRead < 4) {
                        int r = in.read(lenBuf, bytesRead, 4 - bytesRead);
                        if (r <= 0) return;
                        bytesRead += r;
                    }

                    int frameLen = ByteBuffer.wrap(lenBuf).order(ByteOrder.LITTLE_ENDIAN).getInt();
                    if (frameLen <= 0 || frameLen > 1514) continue;

                    // Read frame
                    bytesRead = 0;
                    while (bytesRead < frameLen) {
                        int r = in.read(frameBuf, bytesRead, frameLen - bytesRead);
                        if (r <= 0) return;
                        bytesRead += r;
                    }

                    byte[] frame = new byte[frameLen];
                    System.arraycopy(frameBuf, 0, frame, 0, frameLen);
                    hub.injectFromTap(frame);
                }
            } catch (Exception e) {
                if (running) {
                    EvansComputerMod.LOGGER.error("TAP reader error", e);
                }
            }
        }, "TAP-Reader");
        readerThread.setDaemon(true);
        readerThread.start();
    }

    /**
     * Send a frame through the TAP device.
     */
    public void sendFrame(byte[] frame) {
        if (!running || helperProcess == null || !helperProcess.isAlive()) return;

        try {
            OutputStream out = helperProcess.getOutputStream();
            byte[] lenBuf = ByteBuffer.allocate(4).order(ByteOrder.LITTLE_ENDIAN).putInt(frame.length).array();
            synchronized (out) {
                out.write(lenBuf);
                out.write(frame);
                out.flush();
            }
        } catch (IOException e) {
            if (running) {
                EvansComputerMod.LOGGER.error("TAP write error", e);
            }
        }
    }

    @Override
    public void close() {
        running = false;

        if (helperProcess != null) {
            try {
                helperProcess.getOutputStream().close();
            } catch (IOException e) {
                // ignore
            }
            helperProcess.destroyForcibly();
            helperProcess = null;
        }

        if (readerThread != null) {
            readerThread.interrupt();
            readerThread = null;
        }

        EvansComputerMod.LOGGER.info("TAP bridge closed: {}", deviceName);
    }

    public String getDeviceName() {
        return deviceName;
    }

    public boolean isRunning() {
        return running && helperProcess != null && helperProcess.isAlive();
    }
}
