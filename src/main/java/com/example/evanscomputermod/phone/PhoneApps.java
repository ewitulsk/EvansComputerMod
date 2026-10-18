package com.example.evanscomputermod.phone;

//? if <=1.21.1 {
import com.example.evanscomputermod.computer.ComputerInstance;
import com.example.evanscomputermod.computer.ui.PhoneUi;
import com.example.evanscomputermod.computer.wasi.ProcessManager;
import com.example.evanscomputermod.radio.cellular.FramedModemModule;
import com.example.evanscomputermod.radio.cellular.SimCardItem;
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
 *   <li>Cellular v2: while a framed modem (a modem module, now a framed radio) and
 *       a written SIM are in, the phone runs {@code celld} ({@link PhoneUi#cellular});
 *       its bars and carrier come back through {@code /dev/uictl cell}.</li>
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
        if (ui != null) ui.cellular(framedModem(phone) && SimCardItem.isIssued(phone.hardware().sim()));
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

    /**
     * Cellular bars 0-4; -1 with no modem. A modem is a framed radio driven by celld
     * (Cellular v2): what celld reported (0 until it has).
     */
    static int cellBars(PhoneHost phone) {
        int best = -1;
        for (int i = 0; i < 2; i++) {
            if (phone.bays().getModule(i) instanceof FramedModemModule) {
                PhoneUi ui = ui(phone);
                best = Math.max(best, ui == null ? 0 : ui.celldBars());
            }
        }
        return best;
    }

    /** A modem module loaded as a framed radio (for celld) is in one of the phone's slots. */
    static boolean framedModem(PhoneHost phone) {
        for (int i = 0; i < 2; i++) if (phone.bays().getModule(i) instanceof FramedModemModule) return true;
        return false;
    }

    /** celld is on a cell (the modem draws transmit power). */
    static boolean cellAttached(PhoneHost phone) {
        PhoneUi ui = ui(phone);
        return ui != null && !ui.celldCarrier().isEmpty();
    }

    /** The phone's speaker was written to: the writer's app is playing (audio exception mode). */
    static void audioWritten(PhoneHost phone) {
        PhoneUi ui = ui(phone);
        if (ui != null) ui.audioActivity(ProcessManager.currentPid());
    }

    /** The phone's microphone was read: a hidden reader lights the MIC pill. */
    static void micRead(PhoneHost phone) {
        PhoneUi ui = ui(phone);
        if (ui != null) ui.micActivity(ProcessManager.currentPid());
    }

    /** A double-tap on the AirPods connected to this phone: play/pause the media session. */
    public static String mediaToggle(PhoneHost phone) {
        PhoneUi ui = ui(phone);
        return ui == null ? null : ui.mediaToggle();
    }
}
//?}
