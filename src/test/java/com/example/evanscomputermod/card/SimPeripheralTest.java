package com.example.evanscomputermod.card;

import static org.junit.jupiter.api.Assertions.*;

import com.example.evanscomputermod.api.peripheral.PeripheralException;
import com.example.evanscomputermod.computer.peripheral.PeripheralValues;
import com.example.evanscomputermod.computer.wasi.ProcessSandbox;
import com.example.evanscomputermod.radio.cellular.SimCard;
import org.junit.jupiter.api.Test;

import java.nio.charset.StandardCharsets;
import java.util.HexFormat;
import java.util.List;
import java.util.Locale;
import java.util.Map;

/**
 * The SIM access peripheral (type {@code sim}) that Cellular v2's celld authenticates
 * through: it answers challenges with the documented SimAuth v1 vectors, and K never
 * appears in anything any of its methods returns (as Java values or as the bytes a
 * program receives), in either hex case or raw.
 */
public class SimPeripheralTest {
    static final String KEY = "00112233445566778899aabbccddeeff";
    static final HexFormat HEX = HexFormat.of();

    private static SimPeripheral withCard() {
        SimCard card = new SimCard("cell-red", 310150000000001L, KEY);
        return new SimPeripheral(SimPeripheral.ofCard(() -> card));
    }

    /** Every method with plausible arguments: what a program can get out of the peripheral. */
    private static List<Object> everyAnswer(SimPeripheral p) {
        List<Object> out = new java.util.ArrayList<>();
        for (String m : p.getMethodNames()) {
            for (Object[] args : List.of(new Object[0], new Object[]{"000102030405060708090a0b0c0d0e0f"},
                    new Object[]{"ffffffffffffffffffffffffffffffff"}, new Object[]{KEY}, new Object[]{"00"})) {
                try {
                    out.add(p.callMethod(null, m, args));
                } catch (PeripheralException | RuntimeException e) {
                    out.add(e.getMessage());
                }
            }
        }
        return out;
    }

    private static void assertNoKey(Object answer) {
        String text = String.valueOf(answer);
        assertFalse(text.toLowerCase(Locale.ROOT).contains(KEY), "K (hex) in " + text);
        byte[] wire = PeripheralValues.ok(answer);
        String raw = new String(wire, StandardCharsets.ISO_8859_1);
        assertFalse(raw.toLowerCase(Locale.ROOT).contains(KEY), "K (hex) in the program's bytes");
        assertFalse(raw.contains(new String(HEX.parseHex(KEY), StandardCharsets.ISO_8859_1)), "K (raw) in the program's bytes");
    }

    @Test
    void neverExposesTheKey() {
        SimPeripheral p = withCard();
        for (String m : p.getMethodNames()) {
            String n = m.toLowerCase(Locale.ROOT);
            assertFalse(n.contains("key") || n.contains("secret"), "a method that sounds like it reads K: " + m);
        }
        List<Object> answers = everyAnswer(p);
        assertTrue(answers.size() >= p.getMethodNames().size());
        for (Object a : answers) assertNoKey(a);
        // Even a challenge equal to K gives only RES and KS.
        @SuppressWarnings("unchecked")
        Map<String, Object> r = (Map<String, Object>) assertDoesNotThrow(() -> p.callMethod(null, "respond", new Object[]{KEY}));
        assertEquals(java.util.Set.of("res", "ks"), r.keySet());
    }

    @Test
    void answersWithSimAuthV1() throws Exception {
        SimPeripheral p = withCard();
        assertEquals("sim", p.getType());
        assertEquals("cellular", ProcessSandbox.peripheralPermission("sim"), "apps need the cellular permission");
        assertEquals(true, p.callMethod(null, "present", new Object[0]));
        assertEquals(Map.of("carrier", "cell-red", "imsi", "310150000000001"), p.callMethod(null, "identity", new Object[0]));
        @SuppressWarnings("unchecked")
        Map<String, Object> r = (Map<String, Object>) p.callMethod(null, "respond", new Object[]{"000102030405060708090a0b0c0d0e0f"});
        assertEquals("4c455ba887affefa1448bea727baa584", r.get("res"));
        assertEquals("3a2295de88d4ea04764491a508856d342f71ed47a7415c5bffeabd934308713a", r.get("ks"));
        PeripheralException bad = assertThrows(PeripheralException.class, () -> p.callMethod(null, "respond", new Object[]{"0011"}));
        assertFalse(bad.getMessage().isEmpty());
    }

    @Test
    void anEmptyTrayAnswersNothing() throws Exception {
        SimPeripheral p = new SimPeripheral(SimPeripheral.ofCard(() -> null));
        assertEquals(false, p.callMethod(null, "present", new Object[0]));
        assertNull(p.callMethod(null, "identity", new Object[0]));
        PeripheralException e = assertThrows(PeripheralException.class,
                () -> p.callMethod(null, "respond", new Object[]{"000102030405060708090a0b0c0d0e0f"}));
        assertEquals("no SIM card", e.getMessage());
    }
}
