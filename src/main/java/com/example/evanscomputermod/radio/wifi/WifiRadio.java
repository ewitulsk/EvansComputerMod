package com.example.evanscomputermod.radio.wifi;

import com.example.evanscomputermod.radio.wifi.mac.LowMac;

/**
 * A Wi-Fi radio as the computer's kernel sees it through the {@code wifi_*} host
 * functions (docs/radio/CONTRACTS.md): the Wi-Fi module in a bay implements it.
 * Version-neutral so {@code ComputerInstance} can look for it on every Minecraft
 * version; only 1.21.1 has an implementation.
 */
public interface WifiRadio {

    /** True while the module is installed, live and in Wi-Fi (not controller) mode. */
    boolean wifiActive();

    /** The low MAC behind this radio. */
    LowMac mac();

    /** Called with a notifier that raises the computer's Wi-Fi IRQ when frames or statuses arrive. */
    default void bindInterrupt(Runnable irq) {
        mac().setRxNotify(irq);
    }
}
