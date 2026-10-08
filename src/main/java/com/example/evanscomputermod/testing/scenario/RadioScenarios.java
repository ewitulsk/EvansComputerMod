package com.example.evanscomputermod.testing.scenario;

//? if <=1.21.1 {
import java.util.LinkedHashMap;
import java.util.Map;

/**
 * Radio &amp; Wireless scenarios (1.21.1), spawnable with
 * {@code /ecm scenario spawn <name>} and reused by {@code RadioTests}.
 * Each radio feature adds its scenarios here (one {@code add(...)} line each).
 */
public final class RadioScenarios {

    public static final Map<String, Scenario> ALL = new LinkedHashMap<>();

    static {
        // Features register their scenarios below, one line each.
        add(WifiScenarios.monitor());
        add(WifiScenarios.wpa2Ping());
    }

    static void add(Scenario s) {
        ALL.put(s.name, s);
    }

    private RadioScenarios() {}
}
//?}
