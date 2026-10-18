package com.example.evanscomputermod.card;

//? if <=1.21.1 {
import com.example.evanscomputermod.api.peripheral.AnnotatedPeripheral;
import com.example.evanscomputermod.api.peripheral.IComputerAccess;
import com.example.evanscomputermod.api.peripheral.IPeripheral;
import com.example.evanscomputermod.api.peripheral.PeripheralException;
import com.example.evanscomputermod.api.peripheral.PeripheralMethod;
import com.example.evanscomputermod.card.applet.BankApplet;
import com.example.evanscomputermod.card.applet.BankPublic;
import com.example.evanscomputermod.card.applet.CardException;
import com.example.evanscomputermod.card.applet.NfcApplet;
import com.example.evanscomputermod.card.applet.NfcPublic;
import com.example.evanscomputermod.radio.cellular.SimCardItem;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.world.item.ItemStack;

import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.concurrent.CopyOnWriteArraySet;

/**
 * A contact card reader as a peripheral (type {@code card_reader}): the Card Reader/Writer
 * block and the card reader bay module, over a {@link CardHolder}. Every method runs on
 * the server thread. The card's applets answer; nothing here returns a secret.
 * <pre>
 * import peripheral
 * r = peripheral.find("card_reader")
 * r.identity()                 # {'kind': 'bank', 'uid': ..., 'applets': ['bank'], 'print': {...}, 'bank': {...}}
 * r.write_bank("bank-red", "4000123412341234", key_hex, "1234", True)
 * r.verify_pin("1234"); r.sign(nonce_hex, 1500)          # {'ctr': 1, 'tag': '...'}
 * r.read_stripe(); r.write_stripe("ALICE", "4000...", "")
 * r.nfc_auth(1, key_hex); r.nfc_read(1)
 * </pre>
 * Events: {@code card_inserted(kind, applets)}, {@code card_removed}, {@code pin_failed(tries_left)},
 * {@code card_locked}. The SIM verbs ({@code write}, {@code is_blank}, {@code respond}) are unchanged.
 */
public final class CardReaderPeripheral extends AnnotatedPeripheral {
    public static final String TYPE = "card_reader";

    private final CardHolder holder;
    private final Set<IComputerAccess> computers = new CopyOnWriteArraySet<>();
    private final CardSession session = new CardSession();

    public CardReaderPeripheral(CardHolder holder) {
        this.holder = holder;
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
        return other instanceof CardReaderPeripheral p && p.holder == holder;
    }

    // ------------------------------------------------------------ events (server thread)

    public void cardInserted(ItemStack card) {
        session.reset();
        String kind = kindName(card);
        for (IComputerAccess c : computers) c.queueEvent("card_inserted", kind, CardItem.applets(card));
    }

    public void cardRemoved() {
        session.reset();
        for (IComputerAccess c : computers) c.queueEvent("card_removed");
    }

    private void event(String name, Object... args) {
        for (IComputerAccess c : computers) c.queueEvent(name, args);
    }

    public static String kindName(ItemStack card) {
        CardKind k = CardItem.kindOf(card);
        return k != null ? k.id() : BuiltInRegistries.ITEM.getKey(card.getItem()).toString();
    }

    // ------------------------------------------------------------ helpers

    private ItemStack card() throws PeripheralException {
        ItemStack c = holder.card();
        if (c.isEmpty()) throw new PeripheralException("no card in the reader");
        return c;
    }

    private BankApplet bank() throws PeripheralException {
        BankApplet b = CardItem.bank(card());
        if (b == null) throw new PeripheralException("the card has no bank chip");
        return b;
    }

    private NfcApplet nfc() throws PeripheralException {
        NfcApplet n = CardItem.nfc(card());
        if (n == null) throw new PeripheralException("the card has no NFC applet");
        return n;
    }

    private SmartSim sim() throws PeripheralException {
        SmartSim s = SimCardItem.card(card());
        if (s == null) throw new PeripheralException("the card is not a SIM card");
        return s;
    }

    private static PeripheralException wrap(Exception e) {
        return new PeripheralException(e.getMessage());
    }

    private static int sector(Object o) throws PeripheralException {
        if (o instanceof Number n) return n.intValue();
        try {
            return Integer.parseInt(String.valueOf(o));
        } catch (NumberFormatException e) {
            throw new PeripheralException("a sector number");
        }
    }

    // ------------------------------------------------------------ the card

    @PeripheralMethod(description = "Is a card in the reader?")
    public boolean present() {
        return holder.hasCard();
    }

    @PeripheralMethod(description = "The card: {kind, uid?, applets, print, bank?: {issuer, pan, pin_required, locked, ctr, blank}, nfc?: {written}, sim?: {imsi, carrier, blank}}, or nil with no card. Also type/blank/imsi/carrier for older SIM programs")
    public Map<String, Object> identity() {
        ItemStack c = holder.card();
        if (c.isEmpty()) return null;
        return describe(c);
    }

    public static Map<String, Object> describe(ItemStack c) {
        Map<String, Object> m = new LinkedHashMap<>();
        CardKind k = CardItem.kindOf(c);
        m.put("kind", kindName(c));
        if (CardItem.hasUid(c)) m.put("uid", Hex.ofLong(CardItem.uid(c)));
        m.put("applets", CardItem.applets(c));
        m.put("print", new LinkedHashMap<>(CardItem.fields(c)));
        m.put("printed", CardItem.faces(c) != null);
        if (k != null && k.bank) {
            BankPublic b = c.get(CardContent.BANK_APPLET.get());
            Map<String, Object> bm = new LinkedHashMap<>();
            bm.put("blank", b == null);
            if (b != null) {
                bm.put("issuer", b.issuer());
                bm.put("pan", b.pan());
                bm.put("pin_required", b.pinRequired());
                bm.put("locked", b.locked());
                bm.put("ctr", b.ctr());
            }
            m.put("bank", bm);
        }
        if (k != null && k.nfc) {
            NfcPublic n = c.get(CardContent.NFC_APPLET.get());
            List<Integer> written = new ArrayList<>();
            for (int i = 0; i < 8; i++) if (n != null && n.written(i)) written.add(i);
            m.put("nfc", Map.of("written", written));
        }
        SmartSim s = SimCardItem.card(c);
        if (s != null) {
            Map<String, Object> sm = new LinkedHashMap<>();
            sm.put("blank", s.isBlank());
            SimIdentity id = s.identity();
            if (id != null) {
                sm.put("imsi", id.imsiText());
                sm.put("carrier", id.carrier());
                m.put("imsi", id.imsiText());
                m.put("carrier", id.carrier());
            }
            m.put("sim", sm);
            m.put("type", "sim");
            m.put("blank", s.isBlank());
        } else {
            m.put("type", kindName(c));
            m.put("blank", CardItem.allBlank(c));
        }
        return m;
    }

    @PeripheralMethod(description = "Done with the card: with Auto Eject it is pushed into an adjacent inventory (returns true if it left), otherwise it waits to be pulled out or taken")
    public boolean eject() throws PeripheralException {
        if (!holder.hasCard()) throw new PeripheralException("no card in the reader");
        return holder.release();
    }

    @PeripheralMethod(description = "{present, state ('empty', 'blank', 'written', 'card'), kind, applets, released, auto_eject, pin_ok, nfc_open}")
    public Map<String, Object> status() {
        Map<String, Object> m = new LinkedHashMap<>();
        ItemStack c = holder.card();
        m.put("present", !c.isEmpty());
        m.put("state", c.isEmpty() ? "empty" : CardItem.kindOf(c) == null ? "card" : CardItem.allBlank(c) ? "blank" : "written");
        if (!c.isEmpty()) {
            m.put("kind", kindName(c));
            m.put("applets", CardItem.applets(c));
        }
        m.put("released", holder.released());
        m.put("auto_eject", holder.autoEject());
        m.put("pin_ok", session.pinOk);
        List<Integer> open = new ArrayList<>();
        for (int i = 0; i < 8; i++) if (session.sectorOpen(i)) open.add(i);
        m.put("nfc_open", open);
        return m;
    }

    @PeripheralMethod(description = "Set the printed fields {name, number, issuer, label, ...} (what the printer also sets; at most 8 keys of 64 characters)")
    public Map<String, Object> writeFields(Map<?, ?> fields) throws PeripheralException {
        ItemStack c = card();
        c.set(CardContent.CARD_PRINT.get(), checkFields(fields));
        holder.cardChanged();
        return identity();
    }

    public static Map<String, String> checkFields(Map<?, ?> fields) throws PeripheralException {
        Map<String, String> out = new LinkedHashMap<>();
        if (fields == null) return out;
        if (fields.size() > 8) throw new PeripheralException("at most 8 fields");
        for (Map.Entry<?, ?> e : fields.entrySet()) {
            String k = String.valueOf(e.getKey()), v = e.getValue() == null ? "" : String.valueOf(e.getValue());
            if (k.isEmpty() || k.length() > 16 || v.length() > 64) throw new PeripheralException("field names are 1-16 characters, values at most 64");
            out.put(k, v);
        }
        return out;
    }

    // ------------------------------------------------------------ stripe

    @PeripheralMethod(description = "The magnetic stripe: {t1, t2, t3, corrupted}, or nil for a card without one")
    public Map<String, Object> readStripe() throws PeripheralException {
        Stripe s = CardItem.stripe(card());
        if (s == null) return null;
        Map<String, Object> m = new LinkedHashMap<>();
        m.put("t1", s.t1());
        m.put("t2", s.t2());
        m.put("t3", s.t3());
        m.put("corrupted", s.corrupted());
        return m;
    }

    @PeripheralMethod(description = "Encode the stripe's three tracks (printable ASCII, 64 / 32 / 96 characters at most); clears 'corrupted'")
    public Map<String, Object> writeStripe(String t1, String t2, String t3) throws PeripheralException {
        ItemStack c = card();
        CardKind k = CardItem.kindOf(c);
        if (k == null || !k.stripe) throw new PeripheralException("the card has no magnetic stripe");
        try {
            c.set(CardContent.CARD_STRIPE.get(), new Stripe(t1, t2, t3, false));
        } catch (IllegalArgumentException e) {
            throw wrap(e);
        }
        holder.cardChanged();
        return readStripe();
    }

    // ------------------------------------------------------------ bank chip

    @PeripheralMethod(description = "Write a blank bank chip: write_bank(issuer, pan, key_hex (32 bytes), pin or nil, pin_required). Once per card. Returns the identity")
    public Map<String, Object> writeBank(String issuer, Object pan, String keyHex, Object pin, Object pinRequired) throws PeripheralException {
        BankApplet b = bank();
        try {
            b.write(issuer, pan == null ? null : String.valueOf(pan), Hex.parse(keyHex, 32, "a bank key"),
                    pin == null ? null : String.valueOf(pin), Boolean.TRUE.equals(pinRequired) || "true".equals(String.valueOf(pinRequired)));
        } catch (IllegalArgumentException | CardException e) {
            throw wrap(e);
        }
        holder.cardChanged();
        return identity();
    }

    @PeripheralMethod(description = "Is the bank chip blank (never written)?")
    public boolean bankBlank() throws PeripheralException {
        return bank().isBlank();
    }

    @PeripheralMethod(description = "Check the card's PIN: true opens a PIN session for this card (until it leaves); false counts a failure, the third locks the chip (event card_locked)")
    public boolean verifyPin(Object pin) throws PeripheralException {
        BankApplet b = bank();
        boolean ok;
        try {
            ok = b.verifyPin(pin == null ? null : String.valueOf(pin));
        } catch (CardException e) {
            throw wrap(e);
        }
        holder.cardChanged();
        session.pinOk = ok;
        if (!ok) {
            int left = b.triesLeft();
            event("pin_failed", left);
            if (left == 0) event("card_locked");
        }
        return ok;
    }

    @PeripheralMethod(description = "Sign: sign(nonce_hex (8 bytes), amount) -> {ctr, tag} (TAG = HMAC-SHA256(K, 'ECM-BANK-TAG' || nonce || ctr || amount)[0..16); ctr is incremented first). Needs verify_pin when the card requires a PIN")
    public Map<String, Object> sign(String nonceHex, Object amount) throws PeripheralException {
        BankApplet b = bank();
        long amt = 0;
        if (amount != null) {
            if (amount instanceof Number n) amt = n.longValue();
            else try {
                amt = Long.parseLong(String.valueOf(amount));
            } catch (NumberFormatException e) {
                throw new PeripheralException("amount is a whole number");
            }
        }
        BankApplet.Signature sig;
        try {
            sig = b.sign(Hex.parse(nonceHex, BankApplet.NONCE_BYTES, "a nonce"), amt, session.pinOk);
        } catch (IllegalArgumentException | CardException e) {
            throw wrap(e);
        }
        holder.cardChanged();
        Map<String, Object> m = new LinkedHashMap<>();
        m.put("ctr", sig.ctr());
        m.put("tag", sig.tagHex());
        return m;
    }

    // ------------------------------------------------------------ NFC applet through the contacts

    @PeripheralMethod(description = "Open an NFC sector through the contacts: nfc_auth(sector, key_hex) runs the 3-pass handshake with the card; true opens the sector for this card")
    public boolean nfcAuth(Object sector, String keyHex) throws PeripheralException {
        NfcApplet n = nfc();
        int s = sector(sector);
        byte[] key;
        try {
            key = Hex.parse(keyHex, 16, "a sector key");
            byte[] r1 = new byte[8];
            new java.security.SecureRandom().nextBytes(r1);
            NfcApplet.Challenge ch = n.challenge(s, r1, new java.security.SecureRandom());
            if (!java.security.MessageDigest.isEqual(ch.m1(), NfcApplet.m1(key, r1, ch.r2()))) return false;
            boolean ok = n.checkM2(s, r1, ch.r2(), NfcApplet.m2(key, r1, ch.r2()));
            if (ok) session.open(s);
            holder.cardChanged();
            return ok;
        } catch (IllegalArgumentException | CardException e) {
            throw wrap(e);
        }
    }

    @PeripheralMethod(description = "Read an open sector (64 bytes, hex)")
    public String nfcRead(Object sector) throws PeripheralException {
        NfcApplet n = nfc();
        int s = sector(sector);
        if (!session.sectorOpen(s)) throw new PeripheralException("sector " + s + " is not open: nfc_auth first");
        try {
            return Hex.of(n.read(s));
        } catch (CardException e) {
            throw wrap(e);
        }
    }

    @PeripheralMethod(description = "Write an open sector: nfc_write(sector, hex of 64 bytes)")
    public boolean nfcWrite(Object sector, String dataHex) throws PeripheralException {
        NfcApplet n = nfc();
        int s = sector(sector);
        if (!session.sectorOpen(s)) throw new PeripheralException("sector " + s + " is not open: nfc_auth first");
        try {
            n.write(s, Hex.parse(dataHex, 64, "sector data"));
        } catch (IllegalArgumentException | CardException e) {
            throw wrap(e);
        }
        holder.cardChanged();
        return true;
    }

    @PeripheralMethod(description = "Rekey an open sector: nfc_set_key(sector, key_hex of 16 bytes)")
    public boolean nfcSetKey(Object sector, String keyHex) throws PeripheralException {
        NfcApplet n = nfc();
        int s = sector(sector);
        if (!session.sectorOpen(s)) throw new PeripheralException("sector " + s + " is not open: nfc_auth first");
        try {
            n.setKey(s, Hex.parse(keyHex, 16, "a sector key"));
        } catch (IllegalArgumentException | CardException e) {
            throw wrap(e);
        }
        holder.cardChanged();
        return true;
    }

    // ------------------------------------------------------------ SIM (unchanged verbs)

    @PeripheralMethod(description = "Is the SIM in the reader blank (never written)?")
    public boolean isBlank() throws PeripheralException {
        return sim().isBlank();
    }

    @PeripheralMethod(description = "Write a blank SIM: write(imsi, carrier, key_hex[, number]) with a 6-15 digit IMSI, a carrier id, a 32-hex-digit key and optionally its phone number (E.164, like +12145550142). Releases the card (Auto Eject pushes it out). Returns the identity")
    public Map<String, Object> write(Object imsi, String carrier, String keyHex, String msisdn) throws PeripheralException {
        SmartSim s = sim();
        try {
            s.write(SimAuth.parseImsi(imsi == null ? null : imsi.toString()), SimAuth.parseCarrier(carrier), SimAuth.parseKey(keyHex), SimAuth.parseMsisdn(msisdn));
        } catch (IllegalArgumentException | SmartSim.CardException e) {
            throw wrap(e);
        }
        CardItem.ensureUid(holder.card());
        Map<String, Object> id = identity();
        holder.cardChanged();
        boolean out = holder.release();
        id.put("ejected", out);
        return id;
    }

    @PeripheralMethod(description = "The SIM's answer to a 16-byte challenge (32 hex digits): {res, ks} in hex (RES 16 bytes, session key material 32 bytes)")
    public Map<String, Object> respond(String challengeHex) throws PeripheralException {
        SmartSim s = sim();
        SimAuth.Response r;
        try {
            r = s.respond(SimAuth.parseChallenge(challengeHex));
        } catch (IllegalArgumentException | SmartSim.CardException e) {
            throw wrap(e);
        }
        Map<String, Object> m = new LinkedHashMap<>();
        m.put("res", r.resHex());
        m.put("ks", r.sessionKeyHex());
        return m;
    }
}
//?}
