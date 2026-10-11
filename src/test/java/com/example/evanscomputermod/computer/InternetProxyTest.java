package com.example.evanscomputermod.computer;

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
                var proxy = new InternetProxy(output::offer)) {
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
        try (var proxy = new InternetProxy(output::offer)) {
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
                var proxy = new InternetProxy(output::offer)) {
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
}
