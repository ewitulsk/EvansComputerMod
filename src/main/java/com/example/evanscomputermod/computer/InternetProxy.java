package com.example.evanscomputermod.computer;

import java.io.IOException;
import java.net.*;
import java.nio.ByteBuffer;
import java.nio.channels.*;
import java.util.*;
import java.util.concurrent.*;
import java.util.function.Consumer;

/** Unprivileged Ethernet-to-host socket gateway. Bounded, nonblocking TCP/UDP flows. */
public final class InternetProxy implements AutoCloseable {
    public static final byte[] MAC = {2, 0, 0, 0, 0, 1};
    private static final int GATEWAY = 0x0a000001, MAX_FLOWS = 256, MAX_BUFFER = 65536;
    private final Consumer<byte[]> output;
    private final ScheduledExecutorService worker =
            Executors.newSingleThreadScheduledExecutor(
                    r -> {
                        Thread t = new Thread(r, "ECM-Internet-Proxy");
                        t.setDaemon(true);
                        return t;
                    });
    private final ArrayBlockingQueue<byte[]> frames = new ArrayBlockingQueue<>(256);
    private final Map<Key, Flow> flows = new HashMap<>();
    private volatile boolean closed;

    private record Key(int source, int sport, int target, int dport, int protocol) {}

    private static final class Flow {
        Key key;
        byte[] mac;
        NetworkChannel channel;
        long touched, seq, receive, lastSent;
        boolean connected, fin, guestFin;
        byte[] pending;
        int pendingFlags, retries;
        byte[] writing = new byte[0];

        void close() {
            try {
                channel.close();
            } catch (IOException ignored) {
            }
        }
    }

    public InternetProxy(Consumer<byte[]> output) {
        this.output = output;
        worker.scheduleWithFixedDelay(this::tick, 0, 10, TimeUnit.MILLISECONDS);
    }

    public void sendFrame(byte[] frame) {
        if (!closed && frame.length <= 1536) frames.offer(frame.clone());
    }

    private void tick() {
        try {
            byte[] frame;
            for (int n = 0; n < 256 && (frame = frames.poll()) != null; n++) receive(frame);
            long now = System.currentTimeMillis();
            for (var it = flows.values().iterator(); it.hasNext(); ) {
                Flow f = it.next();
                try {
                    long timeout =
                            f.key.protocol == 6
                                    ? (f.fin ? 240_000 : f.connected ? 7_440_000 : 20_000)
                                    : 300_000;
                    if (now - f.touched > timeout) {
                        f.close();
                        it.remove();
                        continue;
                    }
                    if (f.channel instanceof DatagramChannel udp) {
                        ByteBuffer b = ByteBuffer.allocate(1472);
                        int count = udp.read(b);
                        if (count > 0) {
                            f.touched = now;
                            sendUdp(
                                    f.mac,
                                    f.key.target,
                                    f.key.source,
                                    f.key.dport,
                                    f.key.sport,
                                    Arrays.copyOf(b.array(), count));
                        }
                    } else if (f.channel instanceof SocketChannel tcp) {
                        if (!f.connected) {
                            if (!tcp.finishConnect()) continue;
                            f.connected = true;
                            sendTcp(f, 18, new byte[0], true);
                        }
                        if (f.writing.length > 0) {
                            ByteBuffer b = ByteBuffer.wrap(f.writing);
                            tcp.write(b);
                            f.writing =
                                    Arrays.copyOfRange(f.writing, b.position(), f.writing.length);
                        }
                        if (f.guestFin && f.writing.length == 0 && !tcp.socket().isOutputShutdown())
                            tcp.shutdownOutput();
                        if (f.pending != null) {
                            if (now - f.lastSent > 1000) {
                                if (f.retries++ >= 10) {
                                    f.close();
                                    it.remove();
                                    continue;
                                }
                                sendTcp(f, f.pendingFlags, f.pending, false);
                            }
                        } else if (!f.fin) {
                            ByteBuffer b = ByteBuffer.allocate(1400);
                            int count = tcp.read(b);
                            if (count > 0) {
                                f.touched = now;
                                sendTcp(f, 24, Arrays.copyOf(b.array(), count), true);
                            } else if (count < 0) {
                                f.fin = true;
                                sendTcp(f, 17, new byte[0], true);
                            }
                        } else if (f.guestFin) {
                            f.close();
                            it.remove();
                        }
                    }
                } catch (IOException e) {
                    if (f.key.protocol == 6) sendTcp(f, 20, new byte[0], false);
                    f.close();
                    it.remove();
                }
            }
        } catch (RuntimeException e) {
            com.example.evanscomputermod.EvansComputerMod.LOGGER.warn(
                    "Internet proxy rejected frame", e);
        }
    }

    private void receive(byte[] frame) {
        if (frame.length < 14) return;
        int type = u16(frame, 12);
        if (type == 0x806) {
            arp(frame);
            return;
        }
        if (type != 0x800 || frame.length < 34) return;
        int ihl = (frame[14] & 15) * 4, total = u16(frame, 16);
        if ((frame[14] >>> 4 & 15) != 4
                || ihl < 20
                || total < ihl
                || 14 + total > frame.length
                || (u16(frame, 20) & 0x3fff) != 0
                || checksum(frame, 14, ihl) != 0) return;
        int proto = frame[23] & 255,
                src = i32(frame, 26),
                dst = i32(frame, 30),
                offset = 14 + ihl,
                len = total - ihl;
        byte[] peer = Arrays.copyOfRange(frame, 6, 12);
        if (proto == 17
                && len >= 8
                && u16(frame, offset + 4) >= 8
                && u16(frame, offset + 4) <= len) {
            int sport = u16(frame, offset), dport = u16(frame, offset + 2);
            len = u16(frame, offset + 4);
            // The gateway never serves DHCP: addresses come from DHCP servers players run
            // (router dhcp-server, dhcpd) or static config. Drop requests instead of proxying them.
            if (sport == 68 && dport == 67) return;
            if (u16(frame, offset + 6) != 0
                    && transportChecksum(
                                    src, dst, 17, Arrays.copyOfRange(frame, offset, offset + len))
                            != 0) return;
            if (!routable(dst)) return;
            Key key = new Key(src, sport, dst, dport, 17);
            Flow f = flows.get(key);
            try {
                if (f == null) {
                    if (flows.size() >= MAX_FLOWS) return;
                    f = new Flow();
                    f.key = key;
                    f.mac = peer;
                    DatagramChannel ch = DatagramChannel.open();
                    ch.configureBlocking(false);
                    ch.connect(address(dst, dport));
                    f.channel = ch;
                    flows.put(key, f);
                }
                f.touched = System.currentTimeMillis();
                ((DatagramChannel) f.channel).write(ByteBuffer.wrap(frame, offset + 8, len - 8));
            } catch (IOException e) {
                if (f != null) {
                    f.close();
                    flows.remove(key);
                }
            }
        } else if (proto == 6 && len >= 20) {
            if (transportChecksum(src, dst, 6, Arrays.copyOfRange(frame, offset, offset + len)) != 0
                    || !routable(dst)) return;
            int header = (frame[offset + 12] >>> 4 & 15) * 4;
            if (header < 20 || header > len) return;
            int flags = frame[offset + 13] & 255;
            long seq = Integer.toUnsignedLong(i32(frame, offset + 4)),
                    ack = Integer.toUnsignedLong(i32(frame, offset + 8));
            Key key = new Key(src, u16(frame, offset), dst, u16(frame, offset + 2), 6);
            Flow f = flows.get(key);
            if ((flags & 4) != 0) {
                if (f != null) f.close();
                flows.remove(key);
                return;
            }
            if (f == null && (flags & 2) != 0) {
                if (flows.size() >= MAX_FLOWS) return;
                f = new Flow();
                f.key = key;
                f.mac = peer;
                f.receive = (seq + 1) & 0xffffffffL;
                f.seq = ThreadLocalRandom.current().nextInt() & 0xffffffffL;
                f.touched = System.currentTimeMillis();
                try {
                    SocketChannel ch = SocketChannel.open();
                    ch.configureBlocking(false);
                    ch.connect(address(dst, key.dport));
                    f.channel = ch;
                    flows.put(key, f);
                } catch (IOException e) {
                    sendTcp(f, 20, new byte[0], false);
                }
                return;
            }
            if (f == null) return;
            f.touched = System.currentTimeMillis();
            if ((flags & 16) != 0 && f.pending != null) {
                long end =
                        (f.seq + f.pending.length + ((f.pendingFlags & 3) != 0 ? 1 : 0))
                                & 0xffffffffL;
                if (ack == end) {
                    f.seq = end;
                    f.pending = null;
                }
            }
            int payload = len - header;
            if (seq == f.receive && payload > 0 && f.writing.length + payload <= MAX_BUFFER) {
                byte[] data = Arrays.copyOf(f.writing, f.writing.length + payload);
                System.arraycopy(frame, offset + header, data, f.writing.length, payload);
                f.writing = data;
                f.receive = (f.receive + payload) & 0xffffffffL;
            }
            if ((flags & 1) != 0 && ((seq + payload) & 0xffffffffL) == f.receive && !f.guestFin) {
                f.receive = (f.receive + 1) & 0xffffffffL;
                f.guestFin = true;
            }
            if (payload > 0 || (flags & 3) != 0) {
                if ((flags & 2) != 0 && f.pending != null)
                    sendTcp(f, f.pendingFlags, f.pending, false);
                else sendTcp(f, 16, new byte[0], false);
            }
        }
    }

    private static boolean routable(int ip) {
        int first = ip >>> 24;
        return first != 0 && first < 224 && first != 127 && (ip & 0xffff0000) != 0xa9fe0000;
    }

    private static InetSocketAddress address(int ip, int port) throws UnknownHostException {
        return new InetSocketAddress(
                InetAddress.getByAddress(
                        new byte[] {
                            (byte) (ip >>> 24), (byte) (ip >>> 16), (byte) (ip >>> 8), (byte) ip
                        }),
                port);
    }

    private void arp(byte[] f) {
        if (f.length < 42
                || u16(f, 14) != 1
                || u16(f, 16) != 0x800
                || f[18] != 6
                || f[19] != 4
                || u16(f, 20) != 1
                || i32(f, 38) != GATEWAY) return;
        byte[] reply = new byte[42];
        System.arraycopy(f, 6, reply, 0, 6);
        System.arraycopy(MAC, 0, reply, 6, 6);
        put16(reply, 12, 0x806);
        put16(reply, 14, 1);
        put16(reply, 16, 0x800);
        reply[18] = 6;
        reply[19] = 4;
        put16(reply, 20, 2);
        System.arraycopy(MAC, 0, reply, 22, 6);
        put32(reply, 28, GATEWAY);
        System.arraycopy(f, 22, reply, 32, 6);
        System.arraycopy(f, 28, reply, 38, 4);
        output.accept(reply);
    }

    private void sendTcp(Flow f, int flags, byte[] payload, boolean pending) {
        byte[] tcp = new byte[20 + payload.length];
        put16(tcp, 0, f.key.dport);
        put16(tcp, 2, f.key.sport);
        put32(tcp, 4, (int) f.seq);
        put32(tcp, 8, (int) f.receive);
        tcp[12] = 0x50;
        tcp[13] = (byte) flags;
        put16(tcp, 14, MAX_BUFFER - f.writing.length - 1);
        System.arraycopy(payload, 0, tcp, 20, payload.length);
        put16(tcp, 16, transportChecksum(f.key.target, f.key.source, 6, tcp));
        if (pending) {
            f.pending = payload;
            f.pendingFlags = flags;
            f.retries = 0;
        }
        if (pending || flags == f.pendingFlags) f.lastSent = System.currentTimeMillis();
        output.accept(ip(f.mac, f.key.target, f.key.source, 6, tcp));
    }

    private void sendUdp(byte[] mac, int src, int dst, int sport, int dport, byte[] payload) {
        byte[] udp = new byte[8 + payload.length];
        put16(udp, 0, sport);
        put16(udp, 2, dport);
        put16(udp, 4, udp.length);
        System.arraycopy(payload, 0, udp, 8, payload.length);
        int check = transportChecksum(src, dst, 17, udp);
        put16(udp, 6, check == 0 ? 65535 : check);
        output.accept(ip(mac, src, dst, 17, udp));
    }

    static byte[] ip(byte[] mac, int src, int dst, int proto, byte[] payload) {
        byte[] f = new byte[34 + payload.length];
        System.arraycopy(mac, 0, f, 0, 6);
        System.arraycopy(MAC, 0, f, 6, 6);
        put16(f, 12, 0x800);
        f[14] = 0x45;
        put16(f, 16, 20 + payload.length);
        f[22] = 64;
        f[23] = (byte) proto;
        put32(f, 26, src);
        put32(f, 30, dst);
        put16(f, 24, checksum(f, 14, 20));
        System.arraycopy(payload, 0, f, 34, payload.length);
        return f;
    }

    static int transportChecksum(int src, int dst, int protocol, byte[] data) {
        byte[] b = new byte[12 + data.length];
        put32(b, 0, src);
        put32(b, 4, dst);
        b[9] = (byte) protocol;
        put16(b, 10, data.length);
        System.arraycopy(data, 0, b, 12, data.length);
        return checksum(b, 0, b.length);
    }

    static int checksum(byte[] b, int offset, int length) {
        long sum = 0;
        for (int i = 0; i < length; i += 2)
            sum += (b[offset + i] & 255) * 256 + (i + 1 < length ? b[offset + i + 1] & 255 : 0);
        while (sum > 65535) sum = (sum & 65535) + (sum >>> 16);
        return (int) ~sum & 65535;
    }

    static int u16(byte[] b, int p) {
        return (b[p] & 255) * 256 + (b[p + 1] & 255);
    }

    static int i32(byte[] b, int p) {
        return u16(b, p) * 65536 + u16(b, p + 2);
    }

    static void put16(byte[] b, int p, int value) {
        b[p] = (byte) (value >>> 8);
        b[p + 1] = (byte) value;
    }

    static void put32(byte[] b, int p, int value) {
        put16(b, p, value >>> 16);
        put16(b, p + 2, value);
    }

    public void close() {
        closed = true;
        worker.shutdown();
        try {
            worker.awaitTermination(2, TimeUnit.SECONDS);
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
        }
        flows.values().forEach(Flow::close);
        flows.clear();
        frames.clear();
    }
}
