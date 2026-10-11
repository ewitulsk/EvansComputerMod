package com.example.evanscomputermod.radio.wifi.ap;

//? if <=1.21.1 {
import com.example.evanscomputermod.radio.api.RadioMedium;
import com.example.evanscomputermod.radio.medium.RadioMediumHooks;

import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.Map;

/**
 * The server's running {@link VirtualStation}s (scenario "phones" and test
 * stations), ticked once per server tick on the server thread and dropped
 * when the server stops. Keys are free-form (scenario name + role).
 */
public final class VirtualStations {
    private static final Map<String, VirtualStation> RUNNING = new LinkedHashMap<>();

    private VirtualStations() {}

    /** Starts {@code s} under {@code key}, replacing (and detaching) any station already there. */
    public static synchronized VirtualStation start(String key, VirtualStation s) {
        VirtualStation old = RUNNING.put(key, s);
        if (old != null) old.detach();
        s.attach(RadioMediumHooks.medium());
        return s;
    }

    public static synchronized void stop(String key) {
        VirtualStation old = RUNNING.remove(key);
        if (old != null) old.detach();
    }

    public static synchronized VirtualStation get(String key) {
        return RUNNING.get(key);
    }

    static synchronized void tick() {
        if (RUNNING.isEmpty()) return;
        RadioMedium m = RadioMediumHooks.medium();
        for (VirtualStation s : new ArrayList<>(RUNNING.values())) {
            s.attach(m);
            if (m != null) s.tick();
        }
    }

    static synchronized void clear() {
        RUNNING.clear();
    }
}
//?}
