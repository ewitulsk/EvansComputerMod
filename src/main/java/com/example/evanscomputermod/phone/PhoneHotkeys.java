package com.example.evanscomputermod.phone;

//? if <=1.21.1 {
import com.example.evanscomputermod.computer.ComputerInstance;
import com.example.evanscomputermod.computer.peripheral.PeripheralEventBus;

import java.util.function.Function;

/** The {@link PhoneHotkeySink} every phone uses, and the default one (peripheral events). */
public final class PhoneHotkeys {
    /** Peripheral event names (docs/gui/EPHONE.md). */
    public static final String EVENT_HOTKEY = "phone_hotkey", EVENT_SWITCH_APP = "phone_switch_app";

    private static volatile Function<PhoneHost, PhoneHotkeySink> factory = p -> peripheralEvents();

    private PhoneHotkeys() {}

    /**
     * Seam for the compositor: the sink phones deliver hotkeys to (looked up on
     * every press, so it applies to running phones at once). Null restores the default.
     */
    public static void setFactory(Function<PhoneHost, PhoneHotkeySink> f) {
        factory = f != null ? f : p -> peripheralEvents();
    }

    static PhoneHotkeySink sinkFor(PhoneHost host) {
        return factory.apply(host);
    }

    private static final PhoneHotkeySink PERIPHERAL_EVENTS = new PhoneHotkeySink() {
        @Override
        public void slot(PhoneHost phone, int slot, boolean down) {
            post(phone, EVENT_HOTKEY, slot, down);
        }

        @Override
        public void switchApp(PhoneHost phone) {
            post(phone, EVENT_SWITCH_APP);
        }
    };

    /** The default: {@code phone_hotkey(slot, down)} / {@code phone_switch_app()} from the {@code phone} peripheral. */
    public static PhoneHotkeySink peripheralEvents() {
        return PERIPHERAL_EVENTS;
    }

    private static void post(PhoneHost phone, String event, Object... args) {
        ComputerInstance c = phone.instance();
        PeripheralEventBus bus = c != null ? c.getPeripheralEvents() : null;
        if (bus != null) bus.post(event, PhonePeripheral.NAME, args);
    }
}
//?}
