package com.example.evanscomputermod.internet;

import com.example.evanscomputermod.api.internet.InternetUplinkProvider;

import java.io.BufferedReader;
import java.io.IOException;
import java.io.InputStreamReader;
import java.net.Inet4Address;
import java.net.NetworkInterface;
import java.net.ServerSocket;
import java.net.Socket;
import java.nio.charset.StandardCharsets;
import java.util.Collections;

/**
 * A one-page plain-HTTP server on this machine for scenarios ("a website on
 * the internet" that doesn't depend on a public site). It listens on the
 * machine's LAN address, and the proxy lets computers reach exactly that
 * address and port while it is open, even with LAN destinations blocked.
 */
final class LocalTestSite implements InternetUplinkProvider.TestSite {
    private final ServerSocket listener;
    private final String url;
    private final long exempt;

    private LocalTestSite(ServerSocket listener, Inet4Address address, byte[] response) {
        this.listener = listener;
        this.url = "http://" + address.getHostAddress() + ":" + listener.getLocalPort() + "/";
        this.exempt = InternetProxy.exemptKey(InternetProxy.i32(address.getAddress(), 0), listener.getLocalPort());
        InternetProxy.EXEMPT.add(exempt);
        Thread worker = new Thread(() -> serve(response), "ECM-Internet-test-site");
        worker.setDaemon(true);
        worker.start();
    }

    static LocalTestSite open(String body) throws IOException {
        Inet4Address address = lanAddress();
        if (address == null) throw new IOException("this machine has no IPv4 LAN address for the test site");
        byte[] content = body.getBytes(StandardCharsets.UTF_8);
        byte[] header = ("HTTP/1.0 200 OK\r\nContent-Type: text/plain\r\nContent-Length: " + content.length
                + "\r\nConnection: close\r\n\r\n").getBytes(StandardCharsets.US_ASCII);
        byte[] response = new byte[header.length + content.length];
        System.arraycopy(header, 0, response, 0, header.length);
        System.arraycopy(content, 0, response, header.length, content.length);
        return new LocalTestSite(new ServerSocket(0), address, response);
    }

    private static Inet4Address lanAddress() throws IOException {
        for (var network : Collections.list(NetworkInterface.getNetworkInterfaces()))
            if (network.isUp() && !network.isLoopback())
                for (var ip : Collections.list(network.getInetAddresses()))
                    if (ip instanceof Inet4Address v4 && !ip.isLinkLocalAddress() && ip.isSiteLocalAddress()) return v4;
        return null;
    }

    private void serve(byte[] response) {
        while (!listener.isClosed()) {
            try (Socket socket = listener.accept()) {
                socket.setSoTimeout(3000);
                var input = new BufferedReader(new InputStreamReader(socket.getInputStream(), StandardCharsets.US_ASCII));
                while (true) {
                    String line = input.readLine();
                    if (line == null || line.isEmpty()) break;
                }
                socket.getOutputStream().write(response);
            } catch (IOException e) {
                if (!listener.isClosed()) InternetSidecarMod.LOGGER.warn("Test site rejected a request", e);
            }
        }
    }

    @Override
    public String url() {
        return url;
    }

    @Override
    public void close() {
        InternetProxy.EXEMPT.remove(exempt);
        try {
            listener.close();
        } catch (IOException ignored) {
        }
    }
}
