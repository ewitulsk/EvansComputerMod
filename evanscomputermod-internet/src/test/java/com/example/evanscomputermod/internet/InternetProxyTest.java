package com.example.evanscomputermod.internet;

import static org.junit.jupiter.api.Assertions.*;

import org.junit.jupiter.api.Test;

import java.net.*;
import java.util.*;
import java.util.concurrent.*;

public class InternetProxyTest {
    private static final byte[] CLIENT = {2, 1, 2, 3, 4, 5};
    private static final int SOURCE = 0x0a000002;

    private static int address() throws Exception {
        for (var n : Collections.list(NetworkInterface.getNetworkInterfaces()))
            if (n.isUp() && !n.isLoopback())
                for (var a : Collections.list(n.getInetAddresses()))
                    if (a instanceof Inet4Address) return InternetProxy.i32(a.getAddress(), 0);
        throw new IllegalStateException("No IPv4 host interface for socket integration test");
    }

    private static byte[] request(int dst, int proto, byte[] payload) {
        byte[] f = InternetProxy.ip(InternetProxy.MAC, SOURCE, dst, proto, payload);
        System.arraycopy(CLIENT, 0, f, 6, 6);
        return f;
    }

    @Test
    void udpBridgesHostSocketAndRejectsBadChecksum() throws Exception {
        var output = new LinkedBlockingQueue<byte[]>();
        int target = address();
        try (var socket = new DatagramSocket(0);
                var proxy = new InternetProxy(output::offer, true)) {
            socket.setSoTimeout(3000);
            byte[] payload = "dns-like-odd-length".getBytes();
            byte[] udp = new byte[8 + payload.length];
            InternetProxy.put16(udp, 0, 51000);
            InternetProxy.put16(udp, 2, socket.getLocalPort());
            InternetProxy.put16(udp, 4, udp.length);
            System.arraycopy(payload, 0, udp, 8, payload.length);
            InternetProxy.put16(udp, 6, InternetProxy.transportChecksum(SOURCE, target, 17, udp));
            proxy.sendFrame(request(target, 17, udp));
            var receive = new DatagramPacket(new byte[1500], 1500);
            socket.receive(receive);
            assertArrayEquals(payload, Arrays.copyOf(receive.getData(), receive.getLength()));
            socket.send(new DatagramPacket(payload, payload.length, receive.getSocketAddress()));
            byte[] reply = output.poll(3, TimeUnit.SECONDS);
            assertNotNull(reply);
            assertEquals(SOURCE, InternetProxy.i32(reply, 30));
            assertEquals(
                    0,
                    InternetProxy.transportChecksum(
                            target, SOURCE, 17, Arrays.copyOfRange(reply, 34, reply.length)));
            assertArrayEquals(payload, Arrays.copyOfRange(reply, 42, reply.length));
            udp[8] ^= 1;
            proxy.sendFrame(request(target, 17, udp));
            socket.setSoTimeout(200);
            assertThrows(SocketTimeoutException.class, () -> socket.receive(receive));
        }
    }

    @Test
    void gatewayIgnoresDhcpDiscover() throws Exception {
        var output = new LinkedBlockingQueue<byte[]>();
        try (var proxy = new InternetProxy(output::offer, false)) {
            byte[] bootp = new byte[244];
            bootp[0] = 1; // BOOTREQUEST
            bootp[1] = 1;
            bootp[2] = 6;
            System.arraycopy(CLIENT, 0, bootp, 28, 6);
            InternetProxy.put32(bootp, 236, 0x63825363);
            bootp[240] = 53; // DHCP message type: DISCOVER
            bootp[241] = 1;
            bootp[242] = 1;
            bootp[243] = (byte) 255;
            byte[] udp = new byte[8 + bootp.length];
            InternetProxy.put16(udp, 0, 68);
            InternetProxy.put16(udp, 2, 67);
            InternetProxy.put16(udp, 4, udp.length);
            System.arraycopy(bootp, 0, udp, 8, bootp.length);
            byte[] f = InternetProxy.ip(InternetProxy.MAC, 0, 0xffffffff, 17, udp);
            System.arraycopy(CLIENT, 0, f, 6, 6);
            proxy.sendFrame(f);
            assertNull(output.poll(500, TimeUnit.MILLISECONDS), "the gateway must not answer DHCP");
        }
    }

    private static byte[] tcp(int target, int port, long seq, long ack, int flags, byte[] payload) {
        byte[] tcp = new byte[20 + payload.length];
        InternetProxy.put16(tcp, 0, 51001);
        InternetProxy.put16(tcp, 2, port);
        InternetProxy.put32(tcp, 4, (int) seq);
        InternetProxy.put32(tcp, 8, (int) ack);
        tcp[12] = 0x50;
        tcp[13] = (byte) flags;
        InternetProxy.put16(tcp, 14, 65535);
        System.arraycopy(payload, 0, tcp, 20, payload.length);
        InternetProxy.put16(tcp, 16, InternetProxy.transportChecksum(SOURCE, target, 6, tcp));
        return request(target, 6, tcp);
    }

    @Test
    void tcpHandshakeRequestReplyAndRetransmission() throws Exception {
        var output = new LinkedBlockingQueue<byte[]>();
        int target = address();
        try (var server = new ServerSocket(0);
                var proxy = new InternetProxy(output::offer, true)) {
            server.setSoTimeout(3000);
            int port = server.getLocalPort();
            proxy.sendFrame(tcp(target, port, 100, 0, 2, new byte[0]));
            try (var socket = server.accept()) {
                socket.setSoTimeout(3000);
                byte[] syn = output.poll(3, TimeUnit.SECONDS);
                assertNotNull(syn);
                assertEquals(18, syn[47] & 255);
                long remote = Integer.toUnsignedLong(InternetProxy.i32(syn, 38));
                proxy.sendFrame(tcp(target, port, 101, remote + 1, 16, new byte[0]));
                byte[] request = "GET / HTTP/1.0\r\n\r\n".getBytes();
                proxy.sendFrame(tcp(target, port, 101, remote + 1, 24, request));
                assertArrayEquals(request, socket.getInputStream().readNBytes(request.length));
                byte[] body = "HTTP/1.0 200 OK\r\n\r\nproxy-test".getBytes();
                socket.getOutputStream().write(body);
                socket.getOutputStream().flush();
                byte[] response = null;
                long deadline = System.currentTimeMillis() + 3000;
                while (System.currentTimeMillis() < deadline) {
                    byte[] f = output.poll(100, TimeUnit.MILLISECONDS);
                    if (f != null && f.length > 54) {
                        response = f;
                        break;
                    }
                }
                assertNotNull(response);
                assertArrayEquals(body, Arrays.copyOfRange(response, 54, response.length));
                assertEquals(
                        0,
                        InternetProxy.transportChecksum(
                                target,
                                SOURCE,
                                6,
                                Arrays.copyOfRange(response, 34, response.length)));
                byte[] repeat = output.poll(2, TimeUnit.SECONDS);
                assertNotNull(repeat);
                assertArrayEquals(response, repeat);
                proxy.sendFrame(
                        tcp(
                                target,
                                port,
                                101 + request.length,
                                remote + 1 + body.length,
                                16,
                                new byte[0]));
            }
        }
    }

    /** A guest that acknowledges what arrives in order and counts it; returns the bytes received. */
    private static byte[] download(InternetProxy proxy, LinkedBlockingQueue<byte[]> output, ServerSocket server, int target, byte[] data, int[] largestBurst) throws Exception {
        int port = server.getLocalPort();
        proxy.sendFrame(tcp(target, port, 100, 0, 2, new byte[0]));
        try (var socket = server.accept()) {
            byte[] syn = output.poll(3, TimeUnit.SECONDS);
            assertNotNull(syn);
            long remote = Integer.toUnsignedLong(InternetProxy.i32(syn, 38));
            long expect = (remote + 1) & 0xffffffffL;
            proxy.sendFrame(tcp(target, port, 101, expect, 16, new byte[0]));
            new Thread(() -> {
                try {
                    socket.getOutputStream().write(data);
                    socket.getOutputStream().flush();
                    socket.shutdownOutput();
                } catch (Exception ignored) {
                }
            }).start();
            var got = new java.io.ByteArrayOutputStream();
            long deadline = System.currentTimeMillis() + 20_000;
            boolean fin = false;
            while (!fin && System.currentTimeMillis() < deadline) {
                // Everything that arrived since the last acknowledgement is one burst.
                List<byte[]> burst = new ArrayList<>();
                byte[] f = output.poll(500, TimeUnit.MILLISECONDS);
                while (f != null) {
                    burst.add(f);
                    f = output.poll(15, TimeUnit.MILLISECONDS);
                }
                int bytes = 0;
                for (byte[] seg : burst) {
                    long seq = Integer.toUnsignedLong(InternetProxy.i32(seg, 38));
                    int len = seg.length - 54;
                    if (seq == expect && len > 0) {
                        got.write(seg, 54, len);
                        expect = (expect + len) & 0xffffffffL;
                        bytes += len;
                    }
                    if ((seg[47] & 1) != 0 && seq == expect) {
                        expect = (expect + 1) & 0xffffffffL;
                        fin = true;
                    }
                }
                largestBurst[0] = Math.max(largestBurst[0], bytes);
                if (!burst.isEmpty()) proxy.sendFrame(tcp(target, port, 101, expect, 16, new byte[0]));
            }
            assertTrue(fin, "the download ended with FIN after " + got.size() + " bytes");
            return got.toByteArray();
        }
    }

    @Test
    void downloadsAreWindowedAndInOrder() throws Exception {
        var output = new LinkedBlockingQueue<byte[]>();
        int target = address();
        byte[] data = new byte[300_000];
        for (int i = 0; i < data.length; i++) data[i] = (byte) (i * 31 + (i >> 8));
        int[] burst = {0};
        long t0 = System.currentTimeMillis();
        try (var server = new ServerSocket(0); var proxy = new InternetProxy(output::offer, true)) {
            assertArrayEquals(data, download(proxy, output, server, target, data, burst));
        }
        long ms = System.currentTimeMillis() - t0;
        assertTrue(burst[0] > InternetProxy.SEGMENT, "several segments travel per acknowledgement: " + burst[0]);
        assertTrue(burst[0] <= InternetProxy.WINDOW, "never more than the window: " + burst[0]);
        System.out.println("PROXY_DOWNLOAD 300000 bytes in " + ms + " ms, largest burst " + burst[0]);
    }

    @Test
    void downloadCapIsHonoured() throws Exception {
        var output = new LinkedBlockingQueue<byte[]>();
        int target = address();
        byte[] data = new byte[60_000];
        java.util.Arrays.fill(data, (byte) 7);
        int[] burst = {0};
        long t0 = System.currentTimeMillis();
        // 100 kB/s: 60 kB takes at least half a second.
        try (var server = new ServerSocket(0); var proxy = new InternetProxy(output::offer, true, 100)) {
            assertArrayEquals(data, download(proxy, output, server, target, data, burst));
        }
        long ms = System.currentTimeMillis() - t0;
        assertTrue(ms >= 450, "capped at 100 kB/s, 60 kB took " + ms + " ms");
    }

    @Test
    void blockedAddressesAreRefusedEvenWhenPublic() throws Exception {
        var output = new LinkedBlockingQueue<byte[]>();
        int target = 0x08080808;
        var before = InternetProxy.BLOCKED;
        try (var proxy = new InternetProxy(output::offer, false)) {
            assertTrue(proxy.routable(target, 443));
            InternetProxy.BLOCKED = Set.of(target);
            assertFalse(proxy.routable(target, 443));
            assertTrue(proxy.routable(0x01010101, 443), "other sites are untouched");
            proxy.sendFrame(tcp(target, 443, 500, 0, 2, new byte[0]));
            byte[] reply = output.poll(3, TimeUnit.SECONDS);
            assertNotNull(reply);
            assertEquals(20, reply[47] & 255, "RST+ACK");
        } finally {
            InternetProxy.BLOCKED = before;
        }
        // Only datacenter and off keep computers away from the repository.
        assertTrue(RepoBlock.resolve("open", "https://localhost").isEmpty());
        assertTrue(RepoBlock.resolve("internet", "https://localhost").isEmpty());
        assertEquals(Set.of(0x7f000001), RepoBlock.resolve("datacenter", "http://127.0.0.1:8080"));
    }

    @Test
    void lanDestinationsBlockedByDefault() throws Exception {
        try (var lan = new InternetProxy(b -> {}, false); var open = new InternetProxy(b -> {}, true)) {
            int[] blocked = {0x0a000005, 0xac100001, 0xac1fffff, 0xc0a80101, 0x64400001, 0x647fffff};
            for (int ip : blocked) {
                assertFalse(lan.routable(ip, 80), Integer.toHexString(ip) + " is a LAN address");
                assertTrue(open.routable(ip, 80), "allowLan reaches " + Integer.toHexString(ip));
            }
            int[] never = {0x7f000001, 0xa9fe0001, 0x00000001, 0xe0000001, 0xffffffff};
            for (int ip : never) {
                assertFalse(lan.routable(ip, 80), Integer.toHexString(ip));
                assertFalse(open.routable(ip, 80), "allowLan never reaches " + Integer.toHexString(ip));
            }
            int[] publicIps = {0x08080808, 0x01010101, 0xac200001, 0xab0f0001, 0x64800001, 0xc0a90001};
            for (int ip : publicIps) assertTrue(lan.routable(ip, 80), Integer.toHexString(ip) + " is public");
            // A test site's own address and port only.
            long key = InternetProxy.exemptKey(0xc0a80105, 8080);
            InternetProxy.EXEMPT.add(key);
            try {
                assertTrue(lan.routable(0xc0a80105, 8080));
                assertFalse(lan.routable(0xc0a80105, 8081));
            } finally {
                InternetProxy.EXEMPT.remove(key);
            }
            assertFalse(lan.routable(0xc0a80105, 8080));
        }
    }

    @Test
    void lanSynIsRefusedWithReset() throws Exception {
        var output = new LinkedBlockingQueue<byte[]>();
        int target = 0xc0a80105;
        try (var proxy = new InternetProxy(output::offer, false)) {
            proxy.sendFrame(tcp(target, 80, 500, 0, 2, new byte[0]));
            byte[] reply = output.poll(3, TimeUnit.SECONDS);
            assertNotNull(reply, "a blocked SYN gets an answer");
            assertEquals(20, reply[47] & 255, "RST+ACK");
            assertEquals(501L, Integer.toUnsignedLong(InternetProxy.i32(reply, 42)), "acknowledges the SYN");
            assertEquals(SOURCE, InternetProxy.i32(reply, 30));
        }
    }

    @Test
    void tapDeviceNamesAreValidated() {
        assertTrue(TapBridge.validDeviceName("tap0"));
        assertTrue(TapBridge.validDeviceName("ecm-tap_1.a"));
        assertTrue(TapBridge.validDeviceName("abcdefghijklmno"));
        assertFalse(TapBridge.validDeviceName("abcdefghijklmnop"), "16 characters is too long");
        assertFalse(TapBridge.validDeviceName(""));
        assertFalse(TapBridge.validDeviceName(null));
        assertFalse(TapBridge.validDeviceName("tap0'; import os"));
        assertFalse(TapBridge.validDeviceName("tap 0"));
        assertFalse(TapBridge.validDeviceName("tap0\n"));
        assertThrows(IllegalArgumentException.class, () -> TapBridge.helperScript("x'),os.system('id')#"));
        assertTrue(TapBridge.helperScript("tap0").contains("b'tap0'"));
        assertThrows(java.io.IOException.class, () -> new TapBridge("bad name", f -> {}));
    }

    @Test
    void tapIsOffUnlessNamed() {
        String proxy = System.getProperty("evanscomputermod.internetProxy");
        String tap = System.getProperty("evanscomputermod.tap");
        try {
            System.setProperty("evanscomputermod.internetProxy", "false");
            System.clearProperty("evanscomputermod.tap");
            assertNull(new SocketUplinkProvider().open(f -> {}), "no proxy and no TAP: nothing opened");
            System.setProperty("evanscomputermod.tap", "bad;name");
            assertNull(new SocketUplinkProvider().open(f -> {}), "an invalid TAP name never starts the helper");
            System.clearProperty("evanscomputermod.tap");
            System.setProperty("evanscomputermod.internetProxy", "true");
            var uplink = new SocketUplinkProvider().open(f -> {});
            assertInstanceOf(InternetProxy.class, uplink, "the proxy alone by default");
            uplink.close();
        } finally {
            if (proxy == null) System.clearProperty("evanscomputermod.internetProxy");
            else System.setProperty("evanscomputermod.internetProxy", proxy);
            if (tap == null) System.clearProperty("evanscomputermod.tap");
            else System.setProperty("evanscomputermod.tap", tap);
        }
    }
}