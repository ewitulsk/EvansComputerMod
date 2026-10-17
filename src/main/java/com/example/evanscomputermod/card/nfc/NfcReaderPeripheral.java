package com.example.evanscomputermod.card.nfc;

//? if <=1.21.1 {
import com.example.evanscomputermod.api.peripheral.AnnotatedPeripheral;
import com.example.evanscomputermod.api.peripheral.IComputerAccess;
import com.example.evanscomputermod.api.peripheral.IPeripheral;
import com.example.evanscomputermod.api.peripheral.PeripheralException;
import com.example.evanscomputermod.api.peripheral.PeripheralMethod;
import com.example.evanscomputermod.card.Hex;

import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.concurrent.CopyOnWriteArraySet;

/**
 * An NFC reader as a peripheral (type {@code nfc_reader}), over an {@link NfcRadio}: the
 * NFC Reader block and the NFC module. Every method runs on the server thread.
 * <pre>
 * n = peripheral.find("nfc_reader")
 * n.set_field(True)
 * for c in n.poll(): print(c["uid"], c["kinds"])      # [{'uid': '…', 'kinds': ['nfc']}]
 * n.select(uid); n.auth(1, key_hex); n.read(1); n.write(1, hex64); n.set_key(1, key_hex)
 * n.raw("0100")                                       # any frame, the raw replies
 * </pre>
 * Events: {@code nfc_seen(uid)} when a UID first answers POLL, {@code nfc_gone(uid)} once
 * it has not for ten ticks, {@code nfc_error(reason)}.
 */
public final class NfcReaderPeripheral extends AnnotatedPeripheral {
    public static final String TYPE = "nfc_reader";

    private final NfcRadio radio;
    private final Object owner;
    private final Set<IComputerAccess> computers = new CopyOnWriteArraySet<>();
    private final java.util.Random rnd = new java.security.SecureRandom();

    public NfcReaderPeripheral(NfcRadio radio, Object owner) {
        this.radio = radio;
        this.owner = owner;
    }

    @Override
    public String getType() {
        return TYPE;
    }

    @Override
    public void attach(IComputerAccess computer) {
        computers.add(computer);
    }

    @Override
    public void detach(IComputerAccess computer) {
        computers.remove(computer);
    }

    @Override
    public boolean isSame(IPeripheral other) {
        return other instanceof NfcReaderPeripheral p && p.owner == owner;
    }

    private void event(String name, Object... args) {
        for (IComputerAccess c : computers) c.queueEvent(name, args);
    }

    /** Once a tick from the owner: age the seen set and post nfc_gone. */
    public void tick(long gameTick) {
        List<Long> gone = new ArrayList<>();
        radio.seen().entrySet().removeIf(e -> {
            if (gameTick - e.getValue() > CardEndpointTracker.EXPIRE_TICKS) {
                gone.add(e.getKey());
                return true;
            }
            return false;
        });
        for (long uid : gone) event("nfc_gone", Hex.ofLong(uid));
    }

    private long tickNow() {
        return owner instanceof NfcReaderBlockEntity be && be.getLevel() != null ? be.getLevel().getGameTime()
                : System.nanoTime() / 50_000_000L;
    }

    // ------------------------------------------------------------ field

    @PeripheralMethod(description = "Energise the field (cards only answer while it is on)")
    public boolean setField(boolean on) {
        radio.setField(on);
        return radio.field();
    }

    @PeripheralMethod(description = "Is the field on?")
    public boolean field() {
        return radio.field();
    }

    @PeripheralMethod(description = "Field power, dBm (0 up to the reader's maximum)")
    public double setPower(double dbm) {
        radio.setTxDbm(dbm);
        return radio.txDbm();
    }

    @PeripheralMethod(description = "{field, power_dbm, max_dbm, selected, rssi}")
    public Map<String, Object> status() {
        Map<String, Object> m = new LinkedHashMap<>();
        m.put("field", radio.field());
        m.put("power_dbm", radio.txDbm());
        m.put("max_dbm", radio.maxDbm());
        m.put("selected", radio.selectedUid() == 0 ? null : Hex.ofLong(radio.selectedUid()));
        m.put("rssi", Double.isNaN(radio.lastRssi()) ? null : radio.lastRssi());
        return m;
    }

    // ------------------------------------------------------------ cards

    @PeripheralMethod(description = "POLL the field (up to 4 rounds): [{uid, kinds}] for every card that answered")
    public List<Map<String, Object>> poll() throws PeripheralException {
        needField();
        Map<Long, Integer> found = radio.poll(4);
        if (found.isEmpty()) {
            StringBuilder sb = new StringBuilder();
            for (CardEndpoint ep : CardEndpointTracker.endpoints())
                sb.append(String.format(" %s@%.1f,%.1f,%.1f energised=%s rx=%d (%.1f dBm) tx=%d maxTx=%.1f %s", Hex.ofLong(ep.card().uid()), ep.pose().x(), ep.pose().y(), ep.pose().z(),
                        ep.energised(), ep.received, ep.lastRxDbm, ep.answered, ep.maxTxPowerDbm(), radio.debugLink(ep)));
            com.example.evanscomputermod.EvansComputerMod.LOGGER.debug("[nfc] {} polled at {} (game tick {}): nothing; field {} dBm, receptions {}, tracked{}", Hex.ofLong(radio.id().getMostSignificantBits()),
                    radio.pose(), tickNow(), radio.txDbm(), radio.receptions, sb.isEmpty() ? " none" : sb);
        }
        List<Map<String, Object>> out = new ArrayList<>();
        long now = tickNow();
        for (Map.Entry<Long, Integer> e : found.entrySet()) {
            Map<String, Object> m = new LinkedHashMap<>();
            m.put("uid", Hex.ofLong(e.getKey()));
            List<String> kinds = new ArrayList<>();
            if ((e.getValue() & NfcAir.KIND_BANK) != 0) kinds.add("bank");
            if ((e.getValue() & NfcAir.KIND_NFC) != 0) kinds.add("nfc");
            if ((e.getValue() & NfcAir.KIND_SIM) != 0) kinds.add("sim");
            m.put("kinds", kinds);
            out.add(m);
            if (radio.seen().put(e.getKey(), now) == null) event("nfc_seen", Hex.ofLong(e.getKey()));
        }
        return out;
    }

    @PeripheralMethod(description = "Address one card by UID (hex) for auth/read/write/set_key; true if it answered")
    public boolean select(String uidHex) throws PeripheralException {
        needField();
        return radio.select(Hex.toLong(Hex.parse(uidHex, 8, "a UID")));
    }

    @PeripheralMethod(description = "Open a sector of the selected card over the air: auth(sector, key_hex); true if the handshake succeeded")
    public boolean auth(Object sector, String keyHex) throws PeripheralException {
        needField();
        int s = sector(sector);
        String why = radio.auth(s, Hex.parse(keyHex, 16, "a sector key"), rnd);
        if (why != null && !why.equals("the card does not have this key") && !why.equals("authentication failed")) {
            event("nfc_error", why);
            throw new PeripheralException(why);
        }
        if (why != null) event("nfc_error", why);
        return why == null;
    }

    @PeripheralMethod(description = "Read an open sector of the selected card: 64 bytes, hex")
    public String read(Object sector) throws PeripheralException {
        needField();
        NfcAir.Frame r = radio.one(NfcAir.frame(NfcAir.READ, new byte[] {(byte) sector(sector)}));
        if (r == null) throw new PeripheralException("no answer");
        if (r.isError()) throw new PeripheralException(NfcRadio.reason(r));
        return Hex.of(r.body());
    }

    @PeripheralMethod(description = "Write an open sector of the selected card: write(sector, hex of 64 bytes)")
    public boolean write(Object sector, String dataHex) throws PeripheralException {
        needField();
        NfcAir.Frame r = radio.one(NfcAir.frame(NfcAir.WRITE, new byte[] {(byte) sector(sector)}, Hex.parse(dataHex, 64, "sector data")));
        if (r == null) throw new PeripheralException("no answer");
        if (r.isError()) throw new PeripheralException(NfcRadio.reason(r));
        return true;
    }

    @PeripheralMethod(description = "Rekey an open sector of the selected card: set_key(sector, key_hex of 16 bytes)")
    public boolean setKey(Object sector, String keyHex) throws PeripheralException {
        needField();
        NfcAir.Frame r = radio.one(NfcAir.frame(NfcAir.SETKEY, new byte[] {(byte) sector(sector)}, Hex.parse(keyHex, 16, "a sector key")));
        if (r == null) throw new PeripheralException("no answer");
        if (r.isError()) throw new PeripheralException(NfcRadio.reason(r));
        return true;
    }

    @PeripheralMethod(description = "Send any frame (hex: cmd, len, body) and return the raw replies as hex (for your own protocols and for sniffing experiments)")
    public List<String> raw(String hex) throws PeripheralException {
        needField();
        NfcAir.Frame f = NfcAir.decode(Hex.parseAny(hex, "a frame"));
        if (f == null) throw new PeripheralException("a frame is cmd, len, body");
        List<String> out = new ArrayList<>();
        for (NfcAir.Frame r : radio.exchange(f)) out.add(Hex.of(r.encode()));
        return out;
    }

    @PeripheralMethod(description = "RSSI of the last reply, dBm (nil before any)")
    public Double rssi() {
        return Double.isNaN(radio.lastRssi()) ? null : radio.lastRssi();
    }

    private void needField() throws PeripheralException {
        if (!radio.field()) throw new PeripheralException("the field is off: set_field(true) first");
    }

    private static int sector(Object o) throws PeripheralException {
        int s;
        if (o instanceof Number n) s = n.intValue();
        else try {
            s = Integer.parseInt(String.valueOf(o));
        } catch (NumberFormatException e) {
            throw new PeripheralException("a sector number");
        }
        if (s < 0 || s > 7) throw new PeripheralException("sectors are 0-7");
        return s;
    }
}
//?}
