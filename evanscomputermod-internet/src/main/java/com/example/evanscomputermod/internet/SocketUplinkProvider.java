package com.example.evanscomputermod.internet;

import com.example.evanscomputermod.api.internet.InternetUplink;
import com.example.evanscomputermod.api.internet.InternetUplinkProvider;

import java.io.IOException;
import java.util.ArrayList;
import java.util.List;
import java.util.function.Consumer;

/**
 * The Internet Gateway's uplink: the {@link InternetProxy} (host TCP/UDP
 * sockets, every OS) and, only when the server names a device, a Linux
 * {@link TapBridge}. Server launch properties:
 * <ul>
 *   <li>{@code -Devanscomputermod.internetProxy=false}: no socket proxy;
 *   <li>{@code -Devanscomputermod.internet.allowLan=true}: the proxy may also
 *       reach private/LAN addresses (blocked by default);
 *   <li>{@code -Devanscomputermod.tap=<device>}: also bridge to that TAP
 *       device (default {@code none}).
 * </ul>
 */
public final class SocketUplinkProvider implements InternetUplinkProvider {
    @Override
    public int apiVersion() {
        return CURRENT_API_VERSION;
    }

    @Override
    public String name() {
        return "evanscomputermod-internet";
    }

    @Override
    public InternetUplink open(Consumer<byte[]> injector) {
        List<InternetUplink> parts = new ArrayList<>();
        if (Boolean.parseBoolean(System.getProperty("evanscomputermod.internetProxy", "true"))) {
            boolean allowLan = Boolean.getBoolean("evanscomputermod.internet.allowLan");
            parts.add(new InternetProxy(injector, allowLan));
            InternetSidecarMod.LOGGER.info("Internet Gateway socket proxy enabled at 10.0.0.1 ({})",
                    allowLan ? "LAN destinations allowed" : "LAN destinations blocked");
        }
        String tap = System.getProperty("evanscomputermod.tap", "none");
        if (!"none".equals(tap)) {
            if (!TapBridge.validDeviceName(tap)) {
                InternetSidecarMod.LOGGER.warn("TAP device name '{}' rejected (expected {}): TAP mode is off", tap,
                        TapBridge.DEVICE_NAME.pattern());
            } else {
                try {
                    parts.add(new TapBridge(tap, injector));
                } catch (Exception e) {
                    InternetSidecarMod.LOGGER.warn("TAP bridge unavailable ({}): {}", tap, e.getMessage());
                }
            }
        }
        if (parts.isEmpty()) return null;
        if (parts.size() == 1) return parts.get(0);
        return new InternetUplink() {
            @Override
            public void sendFrame(byte[] frame) {
                for (InternetUplink p : parts) p.sendFrame(frame);
            }

            @Override
            public void close() {
                for (InternetUplink p : parts) p.close();
            }
        };
    }

    @Override
    public TestSite openTestSite(String body) throws IOException {
        return LocalTestSite.open(body);
    }
}
