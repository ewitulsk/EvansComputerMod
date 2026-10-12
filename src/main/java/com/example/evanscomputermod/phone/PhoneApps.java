package com.example.evanscomputermod.phone;

//? if <=1.21.1 {
import com.example.evanscomputermod.computer.ComputerInstance;
import com.example.evanscomputermod.computer.ui.PhoneUi;
import com.example.evanscomputermod.computer.wasi.ProcessManager;
import com.example.evanscomputermod.radio.cellular.CellRadio;
import com.example.evanscomputermod.radio.wifi.WifiRadio;
import org.jetbrains.annotations.Nullable;

/**
 * Glue between the ePhone hardware ({@link PhoneHost}) and the phone's GUI
 * side ({@link PhoneUi}, version-neutral): installed once at mod setup as the
 * {@link PhoneHotkeys} sink and a {@link PhoneLifecycle} listener.
 *
 * <ul>
 *   <li>Hotkey slots go to the open app ({@code Hotkey}), the switch-app key to
 *       the shell ({@code Chord}); a phone whose GUI isn't up yet falls back to
 *       the peripheral events of P5a.</li>
 *   <li>Leaving the hand suspends the running apps first (the sleep waits up to
 *       their save deadline); back in hand the open app resumes.</li>
 *   <li>Every tick the status (battery, state, Wi-Fi and cellular bars) is
 *       handed to the shell's status source.</li>
 * </ul>
 */
public final class PhoneApps {
    private static boolean installed;

    private PhoneApps() {}

    /** Install the hotkey sink and the lifecycle listener (idempotent). */
    public static synchronized void install() {
        if (installed) return;
        installed = true;
        PhoneHotkeys.setFactory(host -> SINK);
        PhoneLifecycle.addListener(new PhoneLifecycle.Listener() {
            @Override
            public long willSleep(PhoneHost phone, PhoneState next) {
                PhoneUi ui = ui(phone);
                return ui == null ? 0 : ui.willSleep();
            }

            @Override
            public void stateChanged(PhoneHost phone, PhoneState from, PhoneState to) {
                PhoneUi ui = ui(phone);
                if (ui != null && to == PhoneState.AWAKE) ui.wake();
            }
        });
    }

    /** The GUI side of a phone, or null while it boots (or has no computer). */
    @Nullable
    public static PhoneUi ui(PhoneHost phone) {
        ComputerInstance c = phone.instance();
        return c == null ? null : c.getUiManager().phone();
    }

    private static final PhoneHotkeySink SINK = new PhoneHotkeySink() {
        @Override
        public void slot(PhoneHost phone, int slot, boolean down) {
            PhoneUi ui = ui(phone);
            if (ui != null) ui.hotkey(slot, down);
            else PhoneHotkeys.peripheralEvents().slot(phone, slot, down);
        }

        @Override
        public void switchApp(PhoneHost phone) {
            PhoneUi ui = ui(phone);
            if (ui != null) ui.switchApp();
            else PhoneHotkeys.peripheralEvents().switchApp(phone);
        }
    };

    /** Once per server tick for an attached phone: keep its status source current. */
    static void tick(PhoneHost phone) {
        PhoneUi ui = ui(phone);
        if (ui != null && phone.statusSourceFor() != ui) {
            phone.statusSourceSet(ui);
            ui.setStatus(new PhoneUi.Status() {
                @Override public double battery() { return phone.batteryPercent(); }
                @Override public String state() { return phone.state().id(); }
                @Override public int wifiBars() { return PhoneApps.wifiBars(phone); }
                @Override public int cellBars() { return PhoneApps.cellBars(phone); }
            });
        }
    }

    /** Wi-Fi bars 0-4 from the joined access point's signal; -1 with no Wi-Fi module switched on. */
    static int wifiBars(PhoneHost phone) {
        int best = -1;
        for (int i = 0; i < 2; i++) {
            if (phone.bays().getModule(i) instanceof WifiRadio w && w.wifiActive()) {
                double dbm = w.mac().bssSignalDbm();
                int bars = Double.isNaN(dbm) ? 0 : dbm >= -55 ? 4 : dbm >= -65 ? 3 : dbm >= -75 ? 2 : dbm >= -85 ? 1 : 0;
                best = Math.max(best, bars);
            }
        }
        return best;
    }

    /** Cellular bars 0-4 from the serving tower (the modem's own {@code bars}); -1 with no modem. */
    static int cellBars(PhoneHost phone) {
        int best = -1;
        for (int i = 0; i < 2; i++) {
            if (phone.bays().getModule(i) instanceof CellRadio c && c.cellActive()) {
                Object b = c.modem().status().get("bars");
                best = Math.max(best, b instanceof Number n ? n.intValue() : 0);
            }
        }
        return best;
    }

    /** The phone's speaker was written to: the writer's app is playing (audio exception mode). */
    static void audioWritten(PhoneHost phone) {
        PhoneUi ui = ui(phone);
        if (ui != null) ui.audioActivity(ProcessManager.currentPid());
    }
}
//?}
