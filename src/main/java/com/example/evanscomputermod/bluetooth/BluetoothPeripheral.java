package com.example.evanscomputermod.bluetooth;

import com.example.evanscomputermod.api.peripheral.AnnotatedPeripheral;
import com.example.evanscomputermod.api.peripheral.PeripheralException;
import com.example.evanscomputermod.api.peripheral.PeripheralMethod;

import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

/**
 * A phone's Bluetooth controller as a peripheral (type {@code bluetooth},
 * docs/BLUETOOTH.md "Programs"): what Settings > Bluetooth and {@code bluetoothctl}
 * use to scan, pair, connect and forget. Pairing and connecting happen over the
 * air, so these calls start them and {@code status}/{@code devices} show how they
 * went.
 */
public final class BluetoothPeripheral extends AnnotatedPeripheral {

    public static final String TYPE = "bluetooth";

    private final PhoneLink link;

    public BluetoothPeripheral(PhoneLink link) {
        this.link = link;
    }

    @Override
    public String getType() {
        return TYPE;
    }

    private static long addr(String s) throws PeripheralException {
        long a = BtAddress.parse(s);
        if (a < 0) throw new PeripheralException("bad address '" + s + "' (like AC:DE:48:00:11:22)");
        return a;
    }

    @PeripheralMethod(mainThread = false,
            description = "{powered, name, address, connected (address or nil), connected_name, audio ('headphones'|'speaker'), battery, rssi_dbm, last}")
    public Map<String, Object> status() {
        Map<String, Object> m = new LinkedHashMap<>();
        m.put("powered", link.powered());
        m.put("name", link.name());
        m.put("address", BtAddress.format(link.addr));
        PhoneLink.Link l = link.link();
        m.put("connected", l == null ? null : BtAddress.format(l.addr));
        String name = null;
        if (l != null) for (PhoneLink.Paired p : link.paired()) if (p.addr() == l.addr) name = p.name();
        m.put("connected_name", name);
        m.put("audio", link.audioRouted() ? "headphones" : "speaker");
        m.put("battery", l == null || l.battery < 0 ? null : l.battery);
        m.put("rssi_dbm", l == null || Double.isNaN(l.rssiDbm) ? null : Math.round(l.rssiDbm));
        m.put("last", link.lastResult());
        return m;
    }

    @PeripheralMethod(mainThread = false, description = "Turn Bluetooth on or off")
    public void setPowered(boolean on) {
        link.setPowered(on);
    }

    @PeripheralMethod(mainThread = false,
            description = "Devices heard advertising lately: [{address, name, rssi_dbm, pairable, paired, connected, battery}]")
    public List<Map<String, Object>> scan() {
        List<Map<String, Object>> out = new ArrayList<>();
        long now = System.currentTimeMillis();
        List<PhoneLink.Paired> paired = link.paired();
        for (PhoneLink.Seen s : link.seen(now)) {
            Map<String, Object> m = new LinkedHashMap<>();
            m.put("address", BtAddress.format(s.addr()));
            m.put("name", s.name());
            m.put("rssi_dbm", Math.round(s.rssiDbm()));
            m.put("pairable", s.adv().pairable());
            m.put("paired", paired.stream().anyMatch(p -> p.addr() == s.addr()));
            m.put("connected", link.connected() == s.addr());
            m.put("battery", s.adv().battery());
            out.add(m);
        }
        return out;
    }

    @PeripheralMethod(mainThread = false,
            description = "Paired devices: [{address, name, connected, connecting, battery}]")
    public List<Map<String, Object>> devices() {
        List<Map<String, Object>> out = new ArrayList<>();
        PhoneLink.Link l = link.link();
        for (PhoneLink.Paired p : link.paired()) {
            Map<String, Object> m = new LinkedHashMap<>();
            m.put("address", BtAddress.format(p.addr()));
            m.put("name", p.name());
            boolean on = l != null && l.addr == p.addr();
            m.put("connected", on);
            m.put("connecting", link.connectingTo(p.addr()));
            m.put("battery", on && l.battery >= 0 ? l.battery : null);
            out.add(m);
        }
        return out;
    }

    @PeripheralMethod(mainThread = false,
            description = "Pair with a device in pairing mode (its case lid open); then it connects. Watch status().last")
    public void pair(String address) throws PeripheralException {
        String err = link.pair(addr(address), System.currentTimeMillis());
        if (err != null) throw new PeripheralException(err);
    }

    @PeripheralMethod(mainThread = false, description = "Connect a paired device (worn, or its case open)")
    public void connect(String address) throws PeripheralException {
        String err = link.connect(addr(address), System.currentTimeMillis());
        if (err != null) throw new PeripheralException(err);
    }

    @PeripheralMethod(mainThread = false, description = "Disconnect the connected device (it stays paired)")
    public void disconnect() {
        link.disconnect();
    }

    @PeripheralMethod(mainThread = false, description = "Forget a paired device")
    public void forget(String address) throws PeripheralException {
        link.forget(addr(address));
    }
}
