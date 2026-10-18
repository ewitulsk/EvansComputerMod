package com.example.evanscomputermod.card;

import com.example.evanscomputermod.api.peripheral.AnnotatedPeripheral;
import com.example.evanscomputermod.api.peripheral.IPeripheral;
import com.example.evanscomputermod.api.peripheral.PeripheralException;
import com.example.evanscomputermod.api.peripheral.PeripheralMethod;
import com.example.evanscomputermod.radio.cellular.SimCard;
import org.jetbrains.annotations.Nullable;

import java.util.LinkedHashMap;
import java.util.Map;
import java.util.function.Supplier;

/**
 * SIM access (peripheral type {@code sim}): how the phone's own software asks the
 * SIM in its tray (or in a Terminal modem's slot) to authenticate, as a real modem
 * asks its SIM over the card interface. Cellular v2's {@code celld} uses it.
 *
 * <pre>
 * import peripheral
 * s = peripheral.find("sim")
 * print(s.identity())                                   # {'carrier': 'cell-red', 'imsi': '310150000000001', 'number': '+12145550142'}
 * print(s.respond("000102030405060708090a0b0c0d0e0f"))  # {'res': '...', 'ks': '...'}
 * </pre>
 *
 * <p>The card answers on the server ({@link SimAuth}); the key K never leaves it: no
 * method returns K or anything K could be recomputed from except the challenge-response
 * pairs a SIM gives every network. {@code respond} does return KS, the 32-byte session
 * key material: that is correct, KS is the phone's (the UE's) session key, which its
 * modem software needs to protect the air link, as in LTE where the USIM hands CK/IK to
 * the terminal. Guarded by the {@code cellular} permission for apps
 * ({@code ProcessSandbox.peripheralPermission}).
 */
public final class SimPeripheral extends AnnotatedPeripheral {
    public static final String TYPE = "sim";

    /** Where the card is: the SIM tray, a modem's slot. */
    public interface Source {
        /** The inserted card's identity, or null with no (written) card. */
        @Nullable
        SimIdentity identity();

        /** The card's answer to {@code challenge}, or null with no card (or no key on this side). */
        @Nullable
        SimAuth.Response respond(byte[] challenge);

        /** The card's IMS AKA answer (docs/TELEPHONY.md), or null. */
        @Nullable
        default SimAuth.Response respondIms(byte[] challenge) {
            return null;
        }
    }

    /** A source over the v1 {@link SimCard} view of the inserted card (server side: it holds the key). */
    public static Source ofCard(Supplier<SimCard> card) {
        return new Source() {
            @Override
            public SimIdentity identity() {
                SimCard s = card.get();
                return s == null ? null : new SimIdentity(s.carrier(), s.imsi(), s.msisdn());
            }

            @Override
            public SimAuth.Response respond(byte[] challenge) {
                SimCard s = card.get();
                return s == null ? null : SimAuth.respond(s.key(), challenge);
            }

            @Override
            public SimAuth.Response respondIms(byte[] challenge) {
                SimCard s = card.get();
                return s == null ? null : SimAuth.respondIms(s.key(), challenge);
            }
        };
    }

    private final Source source;

    public SimPeripheral(Source source) {
        this.source = source;
    }

    @Override
    public String getType() {
        return TYPE;
    }

    @Override
    public boolean isSame(IPeripheral other) {
        return other instanceof SimPeripheral p && p.source == source;
    }

    @PeripheralMethod(description = "Is a written SIM card in?", mainThread = false)
    public boolean present() {
        return source.identity() != null;
    }

    @PeripheralMethod(description = "The SIM's public identity: {carrier, imsi, number} (number: E.164, empty when the card has none), or nil with no card", mainThread = false)
    public Map<String, Object> identity() {
        SimIdentity id = source.identity();
        if (id == null) return null;
        Map<String, Object> m = new LinkedHashMap<>();
        m.put("carrier", id.carrier());
        m.put("imsi", id.imsiText());
        m.put("number", id.msisdn());
        return m;
    }

    @PeripheralMethod(description = "IMS AKA (VoLTE registration): the SIM's answer to a 16-byte challenge (32 hex digits): {res, ks} in hex, with the IMS labels (docs/TELEPHONY.md)", mainThread = false)
    public Map<String, Object> respond_ims(String randHex) throws PeripheralException {
        byte[] rand;
        try {
            rand = SimAuth.parseChallenge(randHex);
        } catch (IllegalArgumentException e) {
            throw new PeripheralException(e.getMessage());
        }
        SimAuth.Response r = source.respondIms(rand);
        if (r == null) throw new PeripheralException("no SIM card");
        Map<String, Object> m = new LinkedHashMap<>();
        m.put("res", r.resHex());
        m.put("ks", r.sessionKeyHex());
        return m;
    }

    @PeripheralMethod(description = "Authenticate: the SIM's answer to a 16-byte challenge RAND (32 hex digits): {res, ks} in hex (RES 16 bytes, KS 32 bytes: the phone's session key material; the key K never leaves the card)", mainThread = false)
    public Map<String, Object> respond(String randHex) throws PeripheralException {
        byte[] rand;
        try {
            rand = SimAuth.parseChallenge(randHex);
        } catch (IllegalArgumentException e) {
            throw new PeripheralException(e.getMessage());
        }
        SimAuth.Response r = source.respond(rand);
        if (r == null) throw new PeripheralException("no SIM card");
        Map<String, Object> m = new LinkedHashMap<>();
        m.put("res", r.resHex());
        m.put("ks", r.sessionKeyHex());
        return m;
    }
}
