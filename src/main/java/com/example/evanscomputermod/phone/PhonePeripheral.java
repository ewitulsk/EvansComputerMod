package com.example.evanscomputermod.phone;

//? if <=1.21.1 {
import com.example.evanscomputermod.api.peripheral.AnnotatedPeripheral;
import com.example.evanscomputermod.api.peripheral.PeripheralMethod;

import java.util.LinkedHashMap;
import java.util.Map;

/**
 * The phone itself, as programs see it: peripheral {@code phone} (type
 * {@code ephone}) on every ePhone's computer. Battery and power state, the
 * display it has, and (until the compositor takes them) the in-hand hotkeys as
 * events {@code phone_hotkey(slot, down)} and {@code phone_switch_app()}.
 *
 * <pre>
 * import peripheral
 * p = peripheral.find("ephone")
 * print(p.battery(), p.state())
 * ev = peripheral.pull_event("phone_hotkey")   # ("phone_hotkey", "phone", 1, True)
 * </pre>
 * See docs/gui/EPHONE.md.
 */
public final class PhonePeripheral extends AnnotatedPeripheral {
    public static final String NAME = "phone", TYPE = "ephone";

    private final PhoneHost host;

    PhonePeripheral(PhoneHost host) {
        this.host = host;
    }

    @Override
    public String getType() {
        return TYPE;
    }

    @PeripheralMethod(description = "Battery charge in percent (0-100); 100 when batteries are off on this server", mainThread = false)
    public double battery() {
        return Math.round(host.batteryPercent() * 10) / 10.0;
    }

    @PeripheralMethod(description = "Power state: awake (held), asleep (hotbar) or off", mainThread = false)
    public String state() {
        return host.state().id();
    }

    @PeripheralMethod(description = "Phone info: battery, state, panel tier, screen size, holder")
    public Map<String, Object> info() {
        Map<String, Object> m = new LinkedHashMap<>();
        m.put("battery", battery());
        m.put("battery_enabled", PhoneConfig.batteryEnabled());
        m.put("state", state());
        var tier = host.hardware().tier();
        m.put("tier", tier == null ? 0 : tier.tier());
        if (tier != null) {
            var mode = tier.modes().phone();
            m.put("width", mode.gfxWidth());
            m.put("height", mode.gfxHeight());
            m.put("cols", mode.textCols());
            m.put("rows", mode.textRows());
            m.put("scale", mode.scale());
        }
        m.put("holder", host.holderName());
        m.put("sim", !host.hardware().sim().isEmpty());
        return m;
    }
}
//?}
