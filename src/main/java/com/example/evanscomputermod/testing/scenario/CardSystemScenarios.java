package com.example.evanscomputermod.testing.scenario;

//? if <=1.21.1 {
import com.example.evanscomputermod.card.CardContent;
import com.example.evanscomputermod.card.CardFaces;
import com.example.evanscomputermod.card.CardItem;
import com.example.evanscomputermod.card.CardKind;
import com.example.evanscomputermod.card.CardReaderBlock;
import com.example.evanscomputermod.card.CardReaderBlockEntity;
import com.example.evanscomputermod.card.Hex;
import com.example.evanscomputermod.card.Stripe;
import com.example.evanscomputermod.card.applet.BankApplet;
import com.example.evanscomputermod.card.applet.BankPublic;
import com.example.evanscomputermod.card.applet.BankSecret;
import com.example.evanscomputermod.card.applet.NfcSecret;
import com.example.evanscomputermod.card.nfc.NfcReaderBlock;
import com.example.evanscomputermod.card.printer.CardPrinterBlock;
import com.example.evanscomputermod.card.printer.CardPrinterBlockEntity;
import com.example.evanscomputermod.card.printer.CardPrinterMenu;
import io.netty.buffer.ByteBufUtil;
import io.netty.buffer.Unpooled;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.network.RegistryFriendlyByteBuf;
import net.minecraft.world.Container;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.item.Items;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.ChestBlock;

import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.HexFormat;
import java.util.List;
import java.util.Locale;
import java.util.regex.Matcher;
import java.util.regex.Pattern;

/**
 * The card system (docs/cards/CARDS.md "Scenarios"), spawnable with {@code /ecm scenario spawn <name>}
 * and run by the {@code ecm_cards} GameTest. Every one is a player at a computer whose east side has the
 * device under test and a chest beyond it (screen facing north, the player stands north):
 * <pre>
 *   y=2  computer (0,2,0)  device (1,2,0)  chest (2,2,0)      floor x -1..4, z -2..5
 * </pre>
 * <ul>
 *   <li>{@code card_print}: a blank bank card and dyes shift-clicked into a Card Printer's screen; the bank.card design typed in;
 *       {@code cardtool print bank.card} paints it (dyes consumed, the printed card lands in the chest with those
 *       faces, fields and stripe); control: a second blank with the red dye gone is refused with {@code no_dye:red}.</li>
 *   <li>{@code card_bank}: a blank bank card in the Card Reader/Writer; {@code cardtool bank write} personalises
 *       it (PIN required); {@code sign} is refused until {@code bank pin}; two signatures with rising counters that
 *       the server's own HMAC matches; three wrong PINs lock the chip (sign refused) while the stripe still reads;
 *       the card's network encoding has no key.</li>
 *   <li>{@code card_stripe}: the stripe written on one card, read back, and cloned onto a blank (the skimmer's
 *       lesson): the clone's tracks are identical.</li>
 *   <li>{@code card_nfc}: an NFC Reader on the computer; an NFC card in the chest next to it answers POLL, the
 *       factory key opens a sector, data is written and the sector rekeyed (the old key then fails); the same card
 *       in a chest four blocks away is not in the field; a card dropped on the floor one block south of it is.</li>
 *   <li>{@code card_reader_module}: a Card Reader Module in the computer's bay takes a card clicked onto the bay,
 *       {@code cardtool info} reads it, and sneak-using the loose module gives the card back.</li>
 * </ul>
 */
public final class CardSystemScenarios {
    static final String PROMPT = PlayerKit.PROMPT;
    static final BlockPos PC = new BlockPos(0, 2, 0), DEVICE = new BlockPos(1, 2, 0), CHEST = new BlockPos(2, 2, 0);
    static final BlockPos FAR_CHEST = new BlockPos(1, 2, 4), DROP = new BlockPos(1, 2, 1);
    static final String PAN = "4000123412341234", NONCE = "0001020304050607";
    static final String KEY_HEX = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";
    static final String SECTOR_KEY = "00112233445566778899aabbccddeeff";
    static final String FACTORY = Hex.of(NfcSecret.FACTORY_KEY);

    private CardSystemScenarios() {}

    private static List<BlockPos> foot() {
        List<BlockPos> f = new ArrayList<>(PlayerKit.box(-1, 0, -2, 4, 0, 5));
        f.addAll(List.of(new BlockPos(0, 1, 0), new BlockPos(1, 1, 0), new BlockPos(2, 1, 0), new BlockPos(1, 1, 1), new BlockPos(1, 1, 4)));
        f.addAll(List.of(DEVICE, CHEST, FAR_CHEST, DROP));
        return f;
    }

    private static Scenario.Decor decor(java.util.function.Consumer<ScenarioRun> build) {
        List<BlockPos> floor = PlayerKit.box(-1, 0, -2, 4, 0, 5);
        List<BlockPos> pedestals = List.of(new BlockPos(0, 1, 0), new BlockPos(1, 1, 0), new BlockPos(2, 1, 0), new BlockPos(1, 1, 1), new BlockPos(1, 1, 4));
        return PlayerKit.decor(foot(), r -> {
            PlayerKit.fill(r, floor, Blocks.SMOOTH_STONE.defaultBlockState());
            PlayerKit.fill(r, pedestals, Blocks.STONE.defaultBlockState());
        }, build, null);
    }

    // ================================================================ card_print

    public static Scenario cardPrint() {
        var b = Scenario.builder("card_print", "a computer prints a bank card's faces, fields and stripe on a Card Printer, paying in dye")
                .asPlayer()
                .host("pc", PC, "-")
                .decor(decor(r -> {
                    r.player().place(CardContent.CARD_PRINTER.get(), r.abs(DEVICE), Direction.NORTH, s -> s.getValue(CardPrinterBlock.FACING) == Direction.NORTH);
                    r.player().place(Blocks.CHEST, r.abs(CHEST), Direction.NORTH, s -> s.getBlock() instanceof ChestBlock);
                    r.say("§7  Card Printer against the computer's east side, a chest beyond it");
                }))
                .note("The design: bank.card (typed in from Python; the Card Designer app saves the same file)");
        PlayerKit.typeBundledFile(b, "pc", "/bank.card", "/evanscomputermod/scenarios/bank.card");
        return b.note("Open the printer (right-click it), set Auto Eject to true; shift-click dyes and a blank bank card into its slots")
                .mutate(r -> setPrinterAutoEject(r, true), "printer screen: Auto Eject = true")
                .mutate(r -> {
                    loadPrinter(r, new ItemStack(Items.WHITE_DYE, 2), new ItemStack(Items.RED_DYE, 1), new ItemStack(Items.BLUE_DYE, 2),
                            new ItemStack(Items.BLACK_DYE, 2), new ItemStack(CardContent.BANK_CARD.get()));
                    CardPrinterBlockEntity be = printer(r);
                    if (be == null || be.cardIn().isEmpty() || be.dyes().size() != 4) throw new IllegalStateException("the printer took no card / dyes: " + (be == null ? "-" : be.dyes()));
                }, "in the printer's screen: 2 white, 1 red, 2 blue and 2 black dye, then a blank bank card, shift-clicked in")
                .send("pc", "cardtool cost bank.card")
                .expect("pc", "^dyes: .*red.*", "the design costs white, red, blue and black dye")
                .expect("pc", PROMPT, "cardtool finished")
                .send("pc", "cardtool print bank.card")
                .expectOrFail("pc", "^printed$", "printed", "^cardtool:")
                .expect("pc", PROMPT, "cardtool finished")
                .await(CardSystemScenarios::printedCardInChest, "the printed card is in the chest with the design's faces, fields and stripe; the dyes were spent", 5_000)
                .note("Control: the red dye is gone, so the same design can't be printed on a second blank")
                .mutate(r -> loadPrinter(r, new ItemStack(CardContent.BANK_CARD.get())), "a second blank bank card shift-clicked in")
                .send("pc", "cardtool print bank.card")
                .expectOrFail("pc", "^cardtool: .*no_dye:red", "refused: no_dye:red", "^printed$")
                .expect("pc", PROMPT, "cardtool finished")
                .timeLimit(60_000)
                .build();
    }

    static CardPrinterBlockEntity printer(ScenarioRun r) {
        return r.level().getBlockEntity(r.abs(DEVICE)) instanceof CardPrinterBlockEntity be ? be : null;
    }

    static void setPrinterAutoEject(ScenarioRun r, boolean on) {
        r.player().openMenu(r.abs(DEVICE), Direction.NORTH, true);
        boolean pressed = r.player().pressMenuButton(on ? CardPrinterMenu.BUTTON_AUTO_EJECT_TRUE : CardPrinterMenu.BUTTON_AUTO_EJECT_FALSE);
        r.player().entity().closeContainer();
        CardPrinterBlockEntity be = printer(r);
        if (!pressed || be == null || be.autoEject() != on) throw new IllegalStateException("couldn't set Auto Eject to " + on + " in the printer screen");
    }

    static List<String> useOnDevice(ScenarioRun r, ItemStack held) {
        return r.player().use(held, r.abs(DEVICE), Direction.NORTH, false);
    }

    /** Open the printer's screen and shift-click each stack from the hotbar into its slots (dye to its colour, a card to the input). */
    static void loadPrinter(ScenarioRun r, ItemStack... stacks) {
        var p = r.player().entity();
        if (r.player().openMenu(r.abs(DEVICE), Direction.NORTH, false) == null) throw new IllegalStateException("the printer screen did not open");
        int hotbar0 = p.containerMenu.slots.size() - 9;
        for (int i = 0; i < stacks.length; i++) {
            p.getInventory().setItem(1 + i, stacks[i]);
            r.player().shiftClick(hotbar0 + 1 + i);
            if (!p.getInventory().getItem(1 + i).isEmpty()) throw new IllegalStateException("the printer did not take " + stacks[i]);
        }
        p.closeContainer();
        p.getInventory().clearContent();
    }

    static List<ItemStack> contents(ScenarioRun r, BlockPos rel) {
        List<ItemStack> out = new ArrayList<>();
        if (r.level().getBlockEntity(r.abs(rel)) instanceof Container c)
            for (int i = 0; i < c.getContainerSize(); i++) if (!c.getItem(i).isEmpty()) out.add(c.getItem(i));
        return out;
    }

    static String printedCardInChest(ScenarioRun r) {
        ItemStack card = ItemStack.EMPTY;
        for (ItemStack s : contents(r, CHEST)) if (CardItem.kindOf(s) == CardKind.BANK) card = s;
        if (card.isEmpty()) return "chest: " + contents(r, CHEST);
        CardFaces f = CardItem.faces(card);
        if (f == null) return "the card in the chest has no faces";
        if (f.palette().length != 4 || CardFaces.texel(f.front(), 0) != 1 || CardFaces.texel(f.front(), 19 * 32 + 31) != 2) {
            r.fail("the printed faces are not the design's: " + f + " texel0=" + CardFaces.texel(f.front(), 0));
            return "failed";
        }
        if (!"ALICE".equals(CardItem.fields(card).get("name"))) {
            r.fail("the printed fields are " + CardItem.fields(card));
            return "failed";
        }
        Stripe st = CardItem.stripe(card);
        if (st == null || !PAN.equals(st.t2())) {
            r.fail("the stripe was not encoded: " + st);
            return "failed";
        }
        CardPrinterBlockEntity be = printer(r);
        if (be == null || be.dyes().containsKey("red") || be.dyes().getOrDefault("white", 0) != 1) {
            r.fail("the dyes were not spent as costed: " + (be == null ? "-" : be.dyes()));
            return "failed";
        }
        r.say("§7  printed: " + f + ", fields " + CardItem.fields(card) + ", dyes left " + be.dyes());
        return null;
    }

    // ================================================================ card_bank

    public static Scenario cardBank() {
        return Scenario.builder("card_bank", "a bank chip is personalised, PIN-gated and counter-signed in the Card Reader/Writer; three wrong PINs lock it")
                .asPlayer()
                .host("pc", PC, "-")
                .decor(decor(CardSystemScenarios::placeReaderAndChest))
                .mutate(r -> useOnDevice(r, new ItemStack(CardContent.BANK_CARD.get())), "right-click the reader with a blank bank card")
                .send("pc", "cardtool info")
                .expect("pc", "^  bank: blank$", "cardtool sees a blank bank chip")
                .expect("pc", PROMPT, "cardtool finished")
                .send("pc", "cardtool bank write bank-red " + PAN + " --key " + KEY_HEX + " --pin 1234 --pin-required")
                .expectOrFail("pc", "^wrote bank card issuer bank-red pan " + PAN + " \\(PIN required\\)$", "chip written", "^cardtool:")
                .expect("pc", PROMPT, "cardtool finished")
                .await(CardSystemScenarios::chipWritten, "the card holds the issuer, PAN and the key (never on the wire)", 2_000)
                .note("Control: no signature without the PIN")
                .send("pc", "cardtool bank sign " + NONCE + " 1500")
                .expectOrFail("pc", "^cardtool: PIN required", "sign refused: PIN required", "^tag ")
                .expect("pc", PROMPT, "cardtool finished")
                .send("pc", "cardtool bank pin 1234")
                .expectOrFail("pc", "^pin ok", "PIN accepted", "^pin wrong")
                .expect("pc", PROMPT, "cardtool finished")
                .send("pc", "cardtool bank sign " + NONCE + " 1500")
                .expectOrFail("pc", "^tag [0-9a-f]{32}$", "signature 1", "^cardtool:")
                .expect("pc", PROMPT, "cardtool finished")
                .await(r -> signatureMatches(r, 1, 1500), "ctr 1 and the tag the server computes from the stored key", 2_000)
                .send("pc", "cardtool bank sign " + NONCE + " 20")
                .expectOrFail("pc", "^tag [0-9a-f]{32}$", "signature 2", "^cardtool:")
                .expect("pc", PROMPT, "cardtool finished")
                .await(r -> signatureMatches(r, 2, 20), "ctr 2: the counter moved", 2_000)
                .send("pc", "cardtool stripe write ALICE " + PAN + " x")
                .expectOrFail("pc", "^wrote stripe", "stripe encoded", "^cardtool:")
                .expect("pc", PROMPT, "cardtool finished")
                .note("Three wrong PINs lock the chip; the stripe still reads")
                .send("pc", "cardtool bank pin 0000")
                .expect("pc", "^pin wrong", "wrong 1")
                .expect("pc", PROMPT, "cardtool finished")
                .send("pc", "cardtool bank pin 0001")
                .expect("pc", "^pin wrong", "wrong 2")
                .expect("pc", PROMPT, "cardtool finished")
                .send("pc", "cardtool bank pin 0002")
                .expect("pc", "^pin wrong", "wrong 3")
                .expect("pc", PROMPT, "cardtool finished")
                .await(CardSystemScenarios::chipLocked, "the chip is locked", 2_000)
                .send("pc", "cardtool bank sign " + NONCE + " 1")
                .expectOrFail("pc", "^cardtool: the bank chip is locked", "sign refused: locked", "^tag ")
                .expect("pc", PROMPT, "cardtool finished")
                .send("pc", "cardtool stripe read")
                .expectOrFail("pc", "^t2 " + PAN + "$", "the stripe still reads", "^cardtool:")
                .expect("pc", PROMPT, "cardtool finished")
                .timeLimit(60_000)
                .build();
    }

    static void placeReaderAndChest(ScenarioRun r) {
        r.player().place(CardContent.CARD_READER.get(), r.abs(DEVICE), Direction.NORTH, s -> s.getValue(CardReaderBlock.FACING) == Direction.NORTH);
        r.player().place(Blocks.CHEST, r.abs(CHEST), Direction.NORTH, s -> s.getBlock() instanceof ChestBlock);
        r.say("§7  Card Reader/Writer against the computer's east side, a chest beyond it");
    }

    static CardReaderBlockEntity reader(ScenarioRun r) {
        return r.level().getBlockEntity(r.abs(DEVICE)) instanceof CardReaderBlockEntity be ? be : null;
    }

    static String lastMatch(String screen, String re) {
        Matcher m = Pattern.compile(re, Pattern.MULTILINE).matcher(screen);
        String last = null;
        while (m.find()) last = m.group(1);
        return last;
    }

    static String chipWritten(ScenarioRun r) {
        CardReaderBlockEntity be = reader(r);
        if (be == null || !be.hasCard()) return "no card in the reader";
        ItemStack c = be.card();
        BankPublic p = c.get(CardContent.BANK_APPLET.get());
        BankSecret s = c.get(CardContent.BANK_SECRET.get());
        if (p == null || s == null || !s.known()) return "not written yet: " + p;
        if (!p.issuer().equals("bank-red") || !p.pan().equals(PAN) || !p.pinRequired() || !s.keyHex().equals(KEY_HEX)) {
            r.fail("the chip holds " + p + " key " + (s.keyHex().equals(KEY_HEX) ? "ok" : "other"));
            return "failed";
        }
        RegistryFriendlyByteBuf buf = new RegistryFriendlyByteBuf(Unpooled.buffer(), r.level().registryAccess());
        ItemStack.OPTIONAL_STREAM_CODEC.encode(buf, c);
        byte[] wire = ByteBufUtil.getBytes(buf);
        String text = new String(wire, StandardCharsets.ISO_8859_1).toLowerCase(Locale.ROOT);
        boolean raw = ByteBufUtil.indexOf(Unpooled.wrappedBuffer(HexFormat.of().parseHex(KEY_HEX)), Unpooled.wrappedBuffer(wire)) >= 0;
        if (text.contains(KEY_HEX) || raw) {
            r.fail("the bank card's network encoding contains its key");
            return "failed";
        }
        r.say("§7  chip: " + p.issuer() + " " + p.panText() + ", key kept on the server only");
        return null;
    }

    static String signatureMatches(ScenarioRun r, long ctr, long amount) {
        CardReaderBlockEntity be = reader(r);
        if (be == null || !be.hasCard()) return "no card";
        BankPublic p = be.card().get(CardContent.BANK_APPLET.get());
        if (p == null || p.ctr() != ctr) return "counter is " + (p == null ? "-" : p.ctr());
        String printedCtr = lastMatch(r.screen("pc"), "^ctr (\\d+)\\s*$"), tag = lastMatch(r.screen("pc"), "^tag ([0-9a-f]{32})\\s*$");
        if (tag == null || printedCtr == null) return "no ctr/tag on the screen";
        String want = Hex.of(BankApplet.tag(HexFormat.of().parseHex(KEY_HEX), HexFormat.of().parseHex(NONCE), ctr, amount));
        if (!printedCtr.equals(String.valueOf(ctr)) || !want.equals(tag)) {
            r.fail("cardtool printed ctr " + printedCtr + " tag " + tag + ", the server computes ctr " + ctr + " tag " + want);
            return "failed";
        }
        r.say("§7  ctr " + ctr + " tag " + tag + " matches HMAC(K, nonce, ctr, " + amount + ")");
        return null;
    }

    static String chipLocked(ScenarioRun r) {
        CardReaderBlockEntity be = reader(r);
        BankPublic p = be == null ? null : be.card().get(CardContent.BANK_APPLET.get());
        if (p == null) return "no written card";
        return p.locked() ? null : "not locked yet (" + p.pinFails() + " fails)";
    }

    // ================================================================ card_stripe

    public static Scenario cardStripe() {
        return Scenario.builder("card_stripe", "a magnetic stripe is written, read and cloned onto a blank: stripes carry no secret")
                .asPlayer()
                .host("pc", PC, "-")
                .decor(decor(CardSystemScenarios::placeReaderAndChest))
                .mutate(r -> setReaderAutoEject(r, true), "reader screen: Auto Eject = true")
                .mutate(r -> useOnDevice(r, new ItemStack(CardContent.BANK_CARD.get())), "right-click the reader with a blank bank card")
                .send("pc", "cardtool stripe write ALICE " + PAN + " LOYALTY=42")
                .expectOrFail("pc", "^wrote stripe", "stripe written", "^cardtool:")
                .expect("pc", PROMPT, "cardtool finished")
                .send("pc", "cardtool stripe read")
                .expectOrFail("pc", "^t3 LOYALTY=42$", "read back", "^cardtool:")
                .expect("pc", PROMPT, "cardtool finished")
                .send("pc", "cardtool eject")
                .expect("pc", PROMPT, "cardtool finished")
                .await(r -> stripedCardsInChest(r, 1), "the original is in the chest", 5_000)
                .note("The skimmer: the tracks go onto a blank")
                .mutate(r -> useOnDevice(r, new ItemStack(CardContent.BANK_CARD.get())), "a blank bank card clicked in")
                .send("pc", "cardtool stripe write ALICE " + PAN + " LOYALTY=42")
                .expectOrFail("pc", "^wrote stripe", "clone written", "^cardtool:")
                .expect("pc", PROMPT, "cardtool finished")
                .send("pc", "cardtool eject")
                .expect("pc", PROMPT, "cardtool finished")
                .await(r -> stripedCardsInChest(r, 2), "two cards with identical tracks in the chest", 5_000)
                .timeLimit(60_000)
                .build();
    }

    static void setReaderAutoEject(ScenarioRun r, boolean on) {
        r.player().openMenu(r.abs(DEVICE), Direction.NORTH, true);
        boolean pressed = r.player().pressMenuButton(on ? com.example.evanscomputermod.card.CardReaderMenu.BUTTON_AUTO_EJECT_TRUE
                : com.example.evanscomputermod.card.CardReaderMenu.BUTTON_AUTO_EJECT_FALSE);
        r.player().entity().closeContainer();
        CardReaderBlockEntity be = reader(r);
        if (!pressed || be == null || be.autoEject() != on) throw new IllegalStateException("couldn't set Auto Eject to " + on + " in the reader screen");
    }

    /** Cards with the tracks in the chest (a clone stacks with its original: identical components). */
    static String stripedCardsInChest(ScenarioRun r, int want) {
        List<Stripe> stripes = new ArrayList<>();
        int count = 0;
        for (ItemStack s : contents(r, CHEST)) {
            Stripe st = CardItem.stripe(s);
            if (st == null || st.isEmpty()) continue;
            stripes.add(st);
            count += s.getCount();
        }
        if (count < want) return "chest: " + contents(r, CHEST);
        for (Stripe st : stripes)
            if (!st.t1().equals("ALICE") || !st.t2().equals(PAN) || !st.t3().equals("LOYALTY=42") || st.corrupted()) {
                r.fail("a stripe differs: " + st);
                return "failed";
            }
        return null;
    }

    // ================================================================ card_nfc

    public static Scenario cardNfc() {
        return Scenario.builder("card_nfc", "an NFC Reader talks to a card in the chest next to it over the near field; four blocks away it hears nothing")
                .asPlayer()
                .host("pc", PC, "-")
                .decor(decor(r -> {
                    r.player().place(CardContent.NFC_READER.get(), r.abs(DEVICE), Direction.NORTH, s -> s.getValue(NfcReaderBlock.FACING) == Direction.NORTH);
                    r.player().place(Blocks.CHEST, r.abs(CHEST), Direction.NORTH, s -> s.getBlock() instanceof ChestBlock);
                    r.player().place(Blocks.CHEST, r.abs(FAR_CHEST), Direction.NORTH, s -> s.getBlock() instanceof ChestBlock);
                    r.say("§7  NFC Reader against the computer's east side; a chest beyond it and another four blocks south");
                }))
                .mutate(r -> putInChest(r, CHEST, new ItemStack(CardContent.NFC_CARD.get())), "a blank NFC card put into the near chest")
                .send("pc", "cardtool nfc field on")
                .expect("pc", "^field on$", "the field is on")
                .expect("pc", PROMPT, "cardtool finished")
                .waitMs(500, "the field finds the card (the tracker registers it, the medium traces the link)")
                .send("pc", "cardtool nfc poll")
                .expectOrFail("pc", "^uid [0-9a-f]{16} kinds nfc$", "the card in the chest answers POLL", "^no cards in the field$")
                .expect("pc", PROMPT, "cardtool finished")
                .sendFn("pc", CardSystemScenarios::selectLine, "cardtool nfc select <the uid it printed>")
                .expectOrFail("pc", "^selected [0-9a-f]{16}$", "selected", "^cardtool:")
                .expect("pc", PROMPT, "cardtool finished")
                .send("pc", "cardtool nfc auth 1 " + FACTORY)
                .expectOrFail("pc", "^sector 1 open$", "the factory key opens sector 1", "^(auth failed|cardtool:)")
                .expect("pc", PROMPT, "cardtool finished")
                .send("pc", "cardtool nfc write 1 " + "5a".repeat(64))
                .expectOrFail("pc", "^wrote sector 1$", "sector 1 written", "^cardtool:")
                .expect("pc", PROMPT, "cardtool finished")
                .send("pc", "cardtool nfc read 1")
                .expectOrFail("pc", "^" + "5a".repeat(64) + "$", "read back", "^cardtool:")
                .expect("pc", PROMPT, "cardtool finished")
                .send("pc", "cardtool nfc setkey 1 " + SECTOR_KEY)
                .expectOrFail("pc", "^sector 1 rekeyed$", "rekeyed", "^cardtool:")
                .expect("pc", PROMPT, "cardtool finished")
                .await(CardSystemScenarios::sectorOnCard, "the card in the chest holds the data and the new key (the chest never left the field)", 2_000)
                .note("Control: the old key no longer opens the sector")
                .sendFn("pc", CardSystemScenarios::selectLine, "cardtool nfc select <uid> again")
                .expectOrFail("pc", "^selected [0-9a-f]{16}$", "selected again", "^cardtool:")
                .expect("pc", PROMPT, "cardtool finished")
                .send("pc", "cardtool nfc auth 1 " + FACTORY)
                .expectOrFail("pc", "^auth failed for sector 1$", "factory key refused", "^sector 1 open$")
                .expect("pc", PROMPT, "cardtool finished")
                .note("Out of range: the card moved to the chest four blocks away")
                .mutate(r -> moveCard(r, CHEST, FAR_CHEST), "take the card out of the near chest, put it in the far one")
                .waitMs(1_000, "the tracker forgets the card")
                .send("pc", "cardtool nfc poll")
                .expectOrFail("pc", "^no cards in the field$", "nothing answers at four blocks", "^uid ")
                .expect("pc", PROMPT, "cardtool finished")
                .note("On the floor: a card dropped one block from the reader is in the field")
                .mutate(r -> dropCard(r, FAR_CHEST, DROP), "take the card out of the far chest and drop it next to the reader")
                .waitMs(500, "the tracker finds it")
                .send("pc", "cardtool nfc poll")
                .expectOrFail("pc", "^uid [0-9a-f]{16} kinds nfc$", "the dropped card answers", "^no cards in the field$")
                .expect("pc", PROMPT, "cardtool finished")
                .timeLimit(60_000)
                .build();
    }

    static void putInChest(ScenarioRun r, BlockPos chest, ItemStack stack) {
        var p = r.player().entity();
        if (r.player().openMenu(r.abs(chest), Direction.NORTH, false) == null) throw new IllegalStateException("the chest did not open");
        p.getInventory().setItem(1, stack);
        int hotbar0 = p.containerMenu.slots.size() - 9;
        r.player().shiftClick(hotbar0 + 1);
        p.closeContainer();
        p.getInventory().clearContent();
        if (contents(r, chest).isEmpty()) throw new IllegalStateException("the card didn't go into the chest");
    }

    static ItemStack takeFromChest(ScenarioRun r, BlockPos chest) {
        var p = r.player().entity();
        if (r.player().openMenu(r.abs(chest), Direction.NORTH, false) == null) throw new IllegalStateException("the chest did not open");
        ItemStack took = ItemStack.EMPTY;
        for (int i = 0; i < 27 && took.isEmpty(); i++)
            if (CardItem.kindOf(p.containerMenu.getSlot(i).getItem()) != null) took = r.player().shiftClick(i);
        p.closeContainer();
        if (took.isEmpty()) throw new IllegalStateException("no card in the chest");
        var inv = p.getInventory();
        ItemStack card = ItemStack.EMPTY;
        for (int i = 0; i < inv.getContainerSize(); i++)
            if (CardItem.kindOf(inv.getItem(i)) != null) { card = inv.getItem(i).copy(); inv.setItem(i, ItemStack.EMPTY); break; }
        if (card.isEmpty()) throw new IllegalStateException("the card didn't reach the inventory");
        return card;
    }

    static void moveCard(ScenarioRun r, BlockPos from, BlockPos to) {
        putInChest(r, to, takeFromChest(r, from));
    }

    /** The player stands at {@code at} and drops the card (the Q key). */
    static void dropCard(ScenarioRun r, BlockPos fromChest, BlockPos at) {
        ItemStack card = takeFromChest(r, fromChest);
        var p = r.player().entity();
        BlockPos a = r.abs(at);
        p.moveTo(a.getX() + 0.5, a.getY(), a.getZ() + 0.5, 180, 60);
        p.getInventory().setItem(0, card);
        p.getInventory().selected = 0;
        var dropped = p.drop(p.getInventory().removeItem(0, 1), false, true);
        if (dropped == null) throw new IllegalStateException("the player dropped nothing");
        dropped.setDeltaMovement(0, 0, 0);
        dropped.setPos(a.getX() + 0.5, a.getY() + 0.1, a.getZ() + 0.5);
    }

    /** The select command for the UID the last poll printed. */
    static String selectLine(ScenarioRun r) {
        String uid = lastMatch(r.screen("pc"), "^uid ([0-9a-f]{16}) kinds");
        if (uid == null) throw new IllegalStateException("no uid on the screen");
        return "cardtool nfc select " + uid;
    }

    static String sectorOnCard(ScenarioRun r) {
        for (ItemStack s : contents(r, CHEST)) {
            NfcSecret n = s.get(CardContent.NFC_SECRET.get());
            if (n == null || !n.known()) continue;
            if (!Hex.of(n.data(1)).equals("5a".repeat(64))) return "sector 1 data is " + Hex.of(n.data(1)).substring(0, 8);
            if (!Hex.of(n.key(1)).equals(SECTOR_KEY)) return "sector 1 key not rekeyed";
            if (!CardItem.hasUid(s)) return "no uid";
            return null;
        }
        return "no touched NFC card in the chest: " + contents(r, CHEST);
    }

    // ================================================================ card_reader_module

    public static Scenario cardReaderModule() {
        return Scenario.builder("card_reader_module", "a Card Reader Module in the computer's bay takes a card clicked onto the bay and reads it")
                .asPlayer()
                .host("pc", PC, "-")
                .decor(decor(r -> r.say("§7  just the computer")))
                .mutate(r -> {
                    List<String> said = r.player().installModule(r.abs(PC), CardContent.CARD_READER_MODULE.get());
                    r.say("§7  expansion card and Card Reader Module clicked into the left bay (" + String.join(" ", said) + ")");
                }, "click a Module Expansion Card, then the Card Reader Module, into the computer's left side")
                .send("pc", "peripherals")
                .expect("pc", "^left_bay_1 +card_reader$", "the module is a card_reader peripheral")
                .expect("pc", PROMPT, "peripherals finished")
                .mutate(r -> {
                    ItemStack card = new ItemStack(CardContent.NFC_CARD.get());
                    List<String> said = r.player().use(card, r.abs(PC), Direction.WEST, ScenarioPlayer.faceCentre(r.abs(PC), Direction.WEST).add(0, 0.25, 0), false);
                    r.say("§7  NFC card clicked onto the bay (" + String.join(" ", said) + ")");
                }, "right-click the bay with an NFC card")
                .send("pc", "cardtool info")
                .expectOrFail("pc", "^left_bay_1: nfc card$", "cardtool reads the card in the module", "^cardtool:")
                .expect("pc", PROMPT, "cardtool finished")
                .send("pc", "cardtool nfc auth 0 " + FACTORY)
                .expectOrFail("pc", "^sector 0 open$", "the contacts reach the NFC applet too", "^(auth failed|cardtool:)")
                .expect("pc", PROMPT, "cardtool finished")
                .note("Sneak-use the bay to take the module out, then sneak-use the loose module in the air: the card comes back")
                .mutate(r -> {
                    r.player().useEmpty(r.abs(PC), Direction.WEST, true);
                    ItemStack module = ItemStack.EMPTY;
                    var inv = r.player().entity().getInventory();
                    for (int i = 0; i < inv.getContainerSize(); i++)
                        if (inv.getItem(i).getItem() instanceof com.example.evanscomputermod.card.CardReaderModuleItem) module = inv.getItem(i).copy();
                    if (module.isEmpty()) throw new IllegalStateException("the module did not come out of the bay");
                    inv.clearContent();
                    r.player().useInAir(module, true);
                    boolean back = false;
                    for (int i = 0; i < inv.getContainerSize(); i++) if (CardItem.kindOf(inv.getItem(i)) == CardKind.NFC) back = true;
                    if (!back) throw new IllegalStateException("the card did not come back from the module");
                    r.say("§7  the card is back in the player's inventory");
                }, "module out of the bay, card out of the module")
                .timeLimit(60_000)
                .build();
    }
}
//?}
