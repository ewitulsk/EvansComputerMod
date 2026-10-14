package com.example.evanscomputermod.card;

//? if <=1.21.1 {
import com.example.evanscomputermod.api.peripheral.AnnotatedPeripheral;
import com.example.evanscomputermod.radio.cellular.SimCardItem;
import com.example.evanscomputermod.api.peripheral.IComputerAccess;
import com.example.evanscomputermod.api.peripheral.IPeripheral;
import com.example.evanscomputermod.api.peripheral.PeripheralException;
import com.example.evanscomputermod.api.peripheral.PeripheralMethod;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.world.item.ItemStack;

import java.util.LinkedHashMap;
import java.util.Map;
import java.util.Set;
import java.util.concurrent.CopyOnWriteArraySet;

/**
 * A Card Reader/Writer as a peripheral (type {@code card_reader}) for the computers next to
 * it. Python:
 * <pre>
 * import peripheral
 * r = peripheral.find("card_reader")
 * if r.present() and r.is_blank():
 *     r.write("310150000000001", "cell-red", "00112233445566778899aabbccddeeff")
 * print(r.identity())        # {'type': 'sim', 'blank': False, 'imsi': '310150000000001', 'carrier': 'cell-red'}
 * ev = peripheral.pull_event()   # ('card_inserted', 'right', 'sim', True) / ('card_removed', 'right')
 * </pre>
 * Every method runs on the server thread. The key of a written SIM can't be read back;
 * {@code respond} shows what the card answers to a challenge.
 */
public final class CardReaderPeripheral extends AnnotatedPeripheral {
    public static final String TYPE = "card_reader";

    private final CardReaderBlockEntity reader;
    private final Set<IComputerAccess> computers = new CopyOnWriteArraySet<>();

    CardReaderPeripheral(CardReaderBlockEntity reader) {
        this.reader = reader;
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
        return other instanceof CardReaderPeripheral p && p.reader == reader;
    }

    // ------------------------------------------------------------ events (server thread)

    void cardInserted(ItemStack card) {
        SmartSim s = SimCardItem.card(card);
        String type = s != null ? "sim" : BuiltInRegistries.ITEM.getKey(card.getItem()).toString();
        boolean blank = s != null && s.isBlank();
        for (IComputerAccess c : computers) c.queueEvent("card_inserted", type, blank);
    }

    void cardRemoved() {
        for (IComputerAccess c : computers) c.queueEvent("card_removed");
    }

    // ------------------------------------------------------------ methods

    private SmartSim sim() throws PeripheralException {
        ItemStack c = reader.card();
        if (c.isEmpty()) throw new PeripheralException("no card in the reader");
        SmartSim s = SimCardItem.card(c);
        if (s == null) throw new PeripheralException("the card is not a SIM card");
        return s;
    }

    @PeripheralMethod(description = "Is a card in the reader?")
    public boolean present() {
        return reader.hasCard();
    }

    @PeripheralMethod(description = "The card: {type, blank, imsi, carrier} (imsi/carrier for a written SIM), or nil with no card")
    public Map<String, Object> identity() {
        ItemStack c = reader.card();
        if (c.isEmpty()) return null;
        Map<String, Object> m = new LinkedHashMap<>();
        SmartSim s = SimCardItem.card(c);
        m.put("type", s != null ? "sim" : BuiltInRegistries.ITEM.getKey(c.getItem()).toString());
        m.put("blank", s != null && s.isBlank());
        SimIdentity id = s == null ? null : s.identity();
        if (id != null) {
            m.put("imsi", id.imsiText());
            m.put("carrier", id.carrier());
        }
        return m;
    }

    @PeripheralMethod(description = "Is the SIM in the reader blank (never written)?")
    public boolean isBlank() throws PeripheralException {
        return sim().isBlank();
    }

    @PeripheralMethod(description = "Write a blank SIM: write(imsi, carrier, key_hex) with a 6-15 digit IMSI, a carrier id and a 32-hex-digit key. Releases the card (Auto Eject Sim pushes it out). Returns the identity")
    public Map<String, Object> write(Object imsi, String carrier, String keyHex) throws PeripheralException {
        SmartSim s = sim();
        try {
            s.write(SimAuth.parseImsi(imsi == null ? null : imsi.toString()), SimAuth.parseCarrier(carrier), SimAuth.parseKey(keyHex));
        } catch (IllegalArgumentException | SmartSim.CardException e) {
            throw new PeripheralException(e.getMessage());
        }
        Map<String, Object> id = identity();
        reader.cardChanged();
        boolean out = reader.release();
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
            throw new PeripheralException(e.getMessage());
        }
        Map<String, Object> m = new LinkedHashMap<>();
        m.put("res", r.resHex());
        m.put("ks", r.sessionKeyHex());
        return m;
    }

    @PeripheralMethod(description = "Done with the card: with Auto Eject Sim it is pushed into an adjacent inventory (returns true if it left), otherwise it waits to be pulled out or taken")
    public boolean eject() throws PeripheralException {
        if (!reader.hasCard()) throw new PeripheralException("no card in the reader");
        return reader.release();
    }

    @PeripheralMethod(description = "{present, state ('empty', 'blank', 'written', 'card'), released, auto_eject}")
    public Map<String, Object> status() {
        Map<String, Object> m = new LinkedHashMap<>();
        m.put("present", reader.hasCard());
        m.put("state", switch (reader.cardState()) {
            case 0 -> "empty";
            case 1 -> "blank";
            case 2 -> "written";
            default -> "card";
        });
        m.put("released", reader.released());
        m.put("auto_eject", reader.autoEject());
        return m;
    }
}
//?}
