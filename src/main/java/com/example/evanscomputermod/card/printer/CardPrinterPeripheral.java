package com.example.evanscomputermod.card.printer;

//? if <=1.21.1 {
import com.example.evanscomputermod.api.peripheral.AnnotatedPeripheral;
import com.example.evanscomputermod.api.peripheral.IComputerAccess;
import com.example.evanscomputermod.api.peripheral.IPeripheral;
import com.example.evanscomputermod.api.peripheral.PeripheralException;
import com.example.evanscomputermod.api.peripheral.PeripheralMethod;
import com.example.evanscomputermod.card.CardFaces;
import com.example.evanscomputermod.card.CardItem;
import com.example.evanscomputermod.card.CardKind;
import com.example.evanscomputermod.card.CardReaderPeripheral;
import com.example.evanscomputermod.card.Hex;
import com.example.evanscomputermod.card.Stripe;
import net.minecraft.world.item.ItemStack;

import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.concurrent.CopyOnWriteArraySet;

/**
 * The Card Printer as a peripheral (type {@code card_printer}). A design is a map
 * {@code {palette: [[r,g,b] | 0xRRGGBB, ...], front: hex, back: hex, fields: {...}, stripe: {t1,t2,t3}?}};
 * the faces are packed 4-bit palette indices, two texels per byte (high nibble first),
 * row-major, the card kind's size. Printing costs one dye per palette colour the design
 * uses (nearest vanilla dye); errors list {@code no_dye:<colour>} for what is missing.
 * Events: {@code card_inserted}, {@code card_removed}, {@code print_done}, {@code print_failed(reason)}.
 */
public final class CardPrinterPeripheral extends AnnotatedPeripheral {
    public static final String TYPE = "card_printer";

    private final CardPrinterBlockEntity printer;
    private final Set<IComputerAccess> computers = new CopyOnWriteArraySet<>();

    CardPrinterPeripheral(CardPrinterBlockEntity printer) {
        this.printer = printer;
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
        return other instanceof CardPrinterPeripheral p && p.printer == printer;
    }

    void cardInserted(ItemStack card) {
        for (IComputerAccess c : computers) c.queueEvent("card_inserted", CardReaderPeripheral.kindName(card), CardItem.applets(card));
    }

    void cardRemoved() {
        for (IComputerAccess c : computers) c.queueEvent("card_removed");
    }

    void printDone() {
        for (IComputerAccess c : computers) c.queueEvent("print_done");
    }

    void printFailed(String why) {
        for (IComputerAccess c : computers) c.queueEvent("print_failed", why);
    }

    // ------------------------------------------------------------ methods

    @PeripheralMethod(description = "Is a card in the input slot?")
    public boolean present() {
        return !printer.cardIn().isEmpty();
    }

    @PeripheralMethod(description = "{present, kind, printing, progress (0-1), out (a printed card waits), dyes: {colour: count}, auto_eject}")
    public Map<String, Object> status() {
        Map<String, Object> m = new LinkedHashMap<>();
        ItemStack in = printer.cardIn();
        m.put("present", !in.isEmpty());
        if (!in.isEmpty()) m.put("kind", CardReaderPeripheral.kindName(in));
        m.put("printing", printer.printing());
        m.put("progress", printer.printing() ? printer.progress() / (double) CardPrinterBlockEntity.PRINT_TICKS : 0.0);
        m.put("out", !printer.cardOut().isEmpty());
        m.put("dyes", printer.dyes());
        m.put("auto_eject", printer.autoEject());
        return m;
    }

    @PeripheralMethod(description = "The card in the input slot (as card_reader.identity), or nil")
    public Map<String, Object> identity() {
        ItemStack in = printer.cardIn();
        return in.isEmpty() ? null : CardReaderPeripheral.describe(in);
    }

    @PeripheralMethod(description = "The dyes a design costs on the card in the slot: {colour: count}, and 'missing': what the printer lacks")
    public Map<String, Object> cost(Map<?, ?> design) throws PeripheralException {
        CardKind k = kind();
        Design d = parse(design, k);
        Map<String, Integer> cost = CardPrinterBlockEntity.cost(d.faces, k);
        Map<String, Object> m = new LinkedHashMap<>(cost);
        m.put("missing", printer.missing(cost));
        return m;
    }

    @PeripheralMethod(description = "Print a design onto the card in the slot: print({palette, front, back, fields?, stripe?}). Consumes dye; the card moves to the output slot when done (event print_done). Errors: no card, already printing, output full, bad design, no_dye:<colour>")
    public boolean print(Map<?, ?> design) throws PeripheralException {
        CardKind k = kind();
        Design d = parse(design, k);
        String why = printer.print(d.faces, d.fields, d.stripe);
        if (why != null) throw new PeripheralException(why);
        return true;
    }

    @PeripheralMethod(description = "Done with the printed card: with Auto Eject it is pushed into an adjacent inventory (true if it left)")
    public boolean eject() throws PeripheralException {
        if (printer.cardOut().isEmpty()) throw new PeripheralException("no printed card waiting");
        return printer.release();
    }

    // ------------------------------------------------------------ the design

    private record Design(CardFaces faces, Map<String, String> fields, Stripe stripe) {}

    private CardKind kind() throws PeripheralException {
        ItemStack in = printer.cardIn();
        if (in.isEmpty()) throw new PeripheralException("no card in the printer");
        CardKind k = CardItem.kindOf(in);
        if (k == null) throw new PeripheralException("not a card");
        return k;
    }

    @SuppressWarnings("unchecked")
    private static Design parse(Map<?, ?> design, CardKind k) throws PeripheralException {
        if (design == null) throw new PeripheralException("a design map");
        Object pal = design.get("palette");
        if (!(pal instanceof List<?> pl) || pl.isEmpty() || pl.size() > CardFaces.MAX_PALETTE)
            throw new PeripheralException("palette: 1-" + CardFaces.MAX_PALETTE + " colours");
        int[] palette = new int[pl.size()];
        for (int i = 0; i < palette.length; i++) {
            Object c = pl.get(i);
            if (c instanceof List<?> rgb && rgb.size() == 3) {
                int r = channel(rgb.get(0)), g = channel(rgb.get(1)), b = channel(rgb.get(2));
                palette[i] = r << 16 | g << 8 | b;
            } else if (c instanceof Number n) palette[i] = n.intValue() & 0xFFFFFF;
            else if (c instanceof String s && s.startsWith("#") && s.length() == 7) palette[i] = Integer.parseInt(s.substring(1), 16);
            else throw new PeripheralException("palette entries are [r, g, b], 0xRRGGBB or '#rrggbb'");
        }
        int n = CardFaces.packedLength(k.texels());
        byte[] front = face(design.get("front"), n, "front"), back = face(design.get("back"), n, "back");
        CardFaces faces;
        try {
            faces = new CardFaces(palette, front, back);
        } catch (IllegalArgumentException e) {
            throw new PeripheralException(e.getMessage());
        }
        Map<String, String> fields = design.get("fields") instanceof Map<?, ?> fm ? CardReaderPeripheral.checkFields(fm) : Map.of();
        Stripe stripe = null;
        if (design.get("stripe") instanceof Map<?, ?> sm) {
            try {
                stripe = new Stripe(str(sm.get("t1")), str(sm.get("t2")), str(sm.get("t3")), false);
            } catch (IllegalArgumentException e) {
                throw new PeripheralException(e.getMessage());
            }
        }
        return new Design(faces, fields, stripe);
    }

    private static int channel(Object o) throws PeripheralException {
        if (!(o instanceof Number n) || n.intValue() < 0 || n.intValue() > 255) throw new PeripheralException("colour channels are 0-255");
        return n.intValue();
    }

    private static String str(Object o) {
        return o == null ? "" : String.valueOf(o);
    }

    private static byte[] face(Object o, int n, String what) throws PeripheralException {
        byte[] b;
        if (o instanceof byte[] raw) b = raw;
        else if (o instanceof String s) {
            try {
                b = Hex.parseAny(s, what);
            } catch (IllegalArgumentException e) {
                throw new PeripheralException(e.getMessage());
            }
        } else throw new PeripheralException(what + ": hex of the packed face");
        if (b.length != n) throw new PeripheralException(what + " is " + n + " bytes for this card (got " + b.length + ")");
        return b;
    }
}
//?}
