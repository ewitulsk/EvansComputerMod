package com.example.evanscomputermod.internet;

import com.example.evanscomputermod.api.internet.InternetUplink;

import java.io.IOException;
import java.io.InputStream;
import java.io.OutputStream;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.util.function.Consumer;
import java.util.regex.Pattern;

/**
 * Optional Linux TAP mode: bridges the Internet Gateway's frames to a host TAP
 * device instead of (or beside) the socket proxy. Off unless the server sets
 * {@code -Devanscomputermod.tap=<device>}.
 *
 * <p>A {@code python3} helper process opens {@code /dev/net/tun} (the ioctl
 * Java can't make) and relays frames on stdin/stdout, each as a 4-byte
 * little-endian length and the frame. The device must already exist and be
 * configured by the server's administrator (see scripts/setup-tap.sh).
 */
public final class TapBridge implements InternetUplink {
    /** Linux interface names: at most 15 characters (IFNAMSIZ - 1), and nothing that could break the helper. */
    static final Pattern DEVICE_NAME = Pattern.compile("^[A-Za-z0-9_.-]{1,15}$");

    private final String deviceName;
    private final Consumer<byte[]> injector;
    private final Process helperProcess;
    private volatile boolean running = true;

    /** Whether {@code name} may be used as a TAP device name. */
    static boolean validDeviceName(String name) {
        return name != null && DEVICE_NAME.matcher(name).matches();
    }

    /** The helper's Python source for a device whose name passed {@link #validDeviceName}. */
    static String helperScript(String deviceName) {
        if (!validDeviceName(deviceName)) throw new IllegalArgumentException("invalid TAP device name: " + deviceName);
        return String.join("\n",
                "import sys, os, struct, fcntl, select, threading",
                "TUNSETIFF = 0x400454ca",
                "IFF_TAP = 0x0002",
                "IFF_NO_PI = 0x1000",
                "fd = os.open('/dev/net/tun', os.O_RDWR)",
                "ifr = struct.pack('16sH22s', b'" + deviceName + "', IFF_TAP | IFF_NO_PI, b'\\x00' * 22)",
                "fcntl.ioctl(fd, TUNSETIFF, ifr)",
                "flags = fcntl.fcntl(fd, fcntl.F_GETFL)",
                "fcntl.fcntl(fd, fcntl.F_SETFL, flags | os.O_NONBLOCK)",
                "sys.stdout.buffer.write(b'OK')",
                "sys.stdout.buffer.flush()",
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
                "os.close(fd)");
    }

    /** Open {@code deviceName} (an existing, configured TAP device); frames from it go to {@code injector}. */
    public TapBridge(String deviceName, Consumer<byte[]> injector) throws IOException {
        if (!validDeviceName(deviceName)) {
            throw new IOException("invalid TAP device name '" + deviceName + "' (expected " + DEVICE_NAME.pattern() + ")");
        }
        this.deviceName = deviceName;
        this.injector = injector;
        helperProcess = new ProcessBuilder("python3", "-c", helperScript(deviceName)).start();

        byte[] ok = helperProcess.getInputStream().readNBytes(2);
        if (ok.length != 2 || ok[0] != 'O' || ok[1] != 'K') {
            helperProcess.destroyForcibly();
            throw new IOException("TAP helper failed to initialize");
        }
        InternetSidecarMod.LOGGER.info("TAP bridge opened: {}", deviceName);

        Thread reader = new Thread(this::readLoop, "ECM-TAP-Reader");
        reader.setDaemon(true);
        reader.start();
    }

    private void readLoop() {
        try {
            InputStream in = helperProcess.getInputStream();
            while (running && helperProcess.isAlive()) {
                byte[] len = in.readNBytes(4);
                if (len.length < 4) return;
                int frameLen = ByteBuffer.wrap(len).order(ByteOrder.LITTLE_ENDIAN).getInt();
                if (frameLen <= 0 || frameLen > 1514) return;
                byte[] frame = in.readNBytes(frameLen);
                if (frame.length < frameLen) return;
                injector.accept(frame);
            }
        } catch (Exception e) {
            if (running) InternetSidecarMod.LOGGER.error("TAP reader error", e);
        }
    }

    @Override
    public void sendFrame(byte[] frame) {
        if (!running || !helperProcess.isAlive() || frame.length > 1514) return;
        try {
            OutputStream out = helperProcess.getOutputStream();
            byte[] len = ByteBuffer.allocate(4).order(ByteOrder.LITTLE_ENDIAN).putInt(frame.length).array();
            synchronized (out) {
                out.write(len);
                out.write(frame);
                out.flush();
            }
        } catch (IOException e) {
            if (running) InternetSidecarMod.LOGGER.error("TAP write error", e);
        }
    }

    @Override
    public void close() {
        running = false;
        try {
            helperProcess.getOutputStream().close();
        } catch (IOException ignored) {
        }
        helperProcess.destroyForcibly();
        InternetSidecarMod.LOGGER.info("TAP bridge closed: {}", deviceName);
    }
}
