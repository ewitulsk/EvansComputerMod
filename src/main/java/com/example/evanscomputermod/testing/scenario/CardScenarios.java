package com.example.evanscomputermod.testing.scenario;

//? if <=1.21.1 {
import com.example.evanscomputermod.card.CardContent;
import com.example.evanscomputermod.card.CardReaderBlock;
import com.example.evanscomputermod.card.CardReaderBlockEntity;
import com.example.evanscomputermod.card.CardReaderMenu;
import com.example.evanscomputermod.card.SimAuth;
import com.example.evanscomputermod.card.SimIdentity;
import com.example.evanscomputermod.card.SimSecret;
import com.example.evanscomputermod.radio.cellular.CellularContent;
import com.example.evanscomputermod.radio.cellular.SimCardItem;
import io.netty.buffer.ByteBufUtil;
import io.netty.buffer.Unpooled;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.network.RegistryFriendlyByteBuf;
import net.minecraft.world.Container;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.ChestBlock;
import net.minecraft.world.level.block.HopperBlock;

import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.HexFormat;
import java.util.List;
import java.util.Locale;
import java.util.regex.Matcher;
import java.util.regex.Pattern;

/**
 * Smart cards (docs/gui/CELLULAR_V2.md, "As built: smart card and card writer"), spawnable with
 * {@code /ecm scenario spawn card_writer} and run by the {@code ecm_cards} GameTest.
 *
 * <p>{@code card_writer}: a player builds a computer with a Card Reader/Writer on its east side
 * and a chest beyond that, and writes SIM cards with {@code simtool}:
 * <ol>
 *   <li>Auto Eject Sim = true (set in the reader's screen): a blank SIM clicked into the reader,
 *       {@code simtool write ... --random} prints the key once and the written SIM lands in the
 *       chest. Control: that SIM, clicked back in, can't be written again; {@code simtool auth}
 *       shows its answer (checked against the server's own computation from the stored key);
 *       {@code simtool eject} pushes it back into the chest.</li>
 *   <li>Auto Eject Sim = false with a hopper under the reader: a fresh blank SIM is held (the
 *       hopper doesn't take an unwritten card); after {@code simtool write} the hopper pulls it.
 *       The SIM's network encoding (what a client receives) has no key.</li>
 *   <li>Hopper input: a hopper on top fed by a chest. A blank SIM dropped into that chest reaches
 *       the reader ({@code simtool wait} sees {@code card_inserted}), is written and pulled out below.</li>
 * </ol>
 * Layout (screen facing north, the player stands north):
 * <pre>
 *        y=4            feed chest (1,4,0)
 *        y=3            hopper     (1,3,0) into the reader
 *   y=2  computer (0,2,0)  reader (1,2,0)  chest (2,2,0)
 *   y=1  stone            hopper (1,1,0)  stone
 *   y=0  floor x -1..3, z -2..1
 * </pre>
 */
public final class CardScenarios {
    static final String PROMPT = PlayerKit.PROMPT;
    static final BlockPos PC = new BlockPos(0, 2, 0), READER = new BlockPos(1, 2, 0), CHEST = new BlockPos(2, 2, 0);
    static final BlockPos HOPPER_BELOW = new BlockPos(1, 1, 0), HOPPER_TOP = new BlockPos(1, 3, 0), FEED = new BlockPos(1, 4, 0);
    static final String KEY_2 = "00112233445566778899aabbccddeeff";
    static final String CHALLENGE = "000102030405060708090a0b0c0d0e0f";
    static final long IMSI_1 = 310_150_000_000_001L, IMSI_2 = 310_150_000_000_002L, IMSI_3 = 310_150_000_000_003L;

    private CardScenarios() {}

    public static Scenario cardWriter() {
        List<BlockPos> floor = PlayerKit.box(-1, 0, -2, 3, 0, 1);
        List<BlockPos> pedestals = List.of(new BlockPos(0, 1, 0), new BlockPos(2, 1, 0));
        List<BlockPos> foot = new ArrayList<>(floor);
        foot.addAll(pedestals);
        foot.addAll(List.of(READER, CHEST, HOPPER_BELOW, HOPPER_TOP, FEED));
        return Scenario.builder("card_writer",
                        "a computer writes SIM cards through a Card Reader/Writer: Auto Eject Sim into a chest, a hopper pulling, a hopper feeding")
                .asPlayer()
                .host("pc", PC, "-")
                .decor(PlayerKit.decor(foot, r -> {
                    PlayerKit.fill(r, floor, Blocks.SMOOTH_STONE.defaultBlockState());
                    PlayerKit.fill(r, pedestals, Blocks.STONE.defaultBlockState());
                }, r -> {
                    r.player().place(CardContent.CARD_READER.get(), r.abs(READER), Direction.NORTH,
                            s -> s.getValue(CardReaderBlock.FACING) == Direction.NORTH);
                    r.player().place(Blocks.CHEST, r.abs(CHEST), Direction.NORTH, s -> s.getBlock() instanceof ChestBlock);
                    r.say("§7  Card Reader/Writer placed against the computer's east side, a chest beyond it");
                }, null))
                .note("Part 1: Auto Eject Sim = true. Open the reader (right-click it empty-handed), set Auto Eject Sim to true")
                .mutate(r -> setAutoEject(r, true), "reader screen: Auto Eject Sim = true")
                .mutate(r -> insert(r, new ItemStack(CellularContent.SIM_CARD.get())), "right-click the reader with a blank SIM card")
                .send("pc", "simtool")
                .expect("pc", "blank SIM$", "simtool sees a blank SIM")
                .expect("pc", PROMPT, "simtool finished")
                .send("pc", "simtool write " + IMSI_1 + " cell-red --random")
                .expectOrFail("pc", "^card ejected into an adjacent inventory$", "written and ejected", "^simtool:")
                .expect("pc", PROMPT, "simtool finished")
                .await(r -> chestHolds(r, IMSI_1), "the written SIM is in the chest, with the key simtool printed", 5_000)
                .note("Control: a written SIM can't be written again")
                .mutate(CardScenarios::takeFromChestAndInsert, "take the SIM out of the chest, right-click the reader with it")
                .send("pc", "simtool write 310150000000009 cell-blue --random")
                .expectOrFail("pc", "^simtool: the card is not blank", "simtool refuses: the card is not blank", "^wrote SIM")
                .expect("pc", PROMPT, "simtool finished")
                .await(r -> readerHolds(r, IMSI_1, "cell-red"), "the card is unchanged (IMSI 1, cell-red)", 2_000)
                .send("pc", "simtool auth " + CHALLENGE)
                .expectOrFail("pc", "^ks  [0-9a-f]{64}$", "the card answers a challenge", "^simtool: (no card|the card is blank|the card has no key)")
                .expect("pc", PROMPT, "simtool finished")
                .await(CardScenarios::authMatchesServer, "its answer is HMAC-SHA256 of the stored key (SimAuth)", 2_000)
                .send("pc", "simtool eject")
                .expectOrFail("pc", "^card ejected into an adjacent inventory$", "simtool eject pushes it out", "^simtool: no card")
                .expect("pc", PROMPT, "simtool finished")
                .await(r -> chestHolds(r, IMSI_1), "back in the chest", 5_000)
                .note("Part 2: Auto Eject Sim = false, a hopper under the reader")
                .mutate(r -> setAutoEject(r, false), "reader screen: Auto Eject Sim = false")
                .mutate(r -> r.player().place(Blocks.HOPPER, r.abs(HOPPER_BELOW), Direction.NORTH,
                        s -> s.getValue(HopperBlock.FACING) == Direction.DOWN), "place a hopper under the reader")
                .mutate(r -> insert(r, new ItemStack(CellularContent.SIM_CARD.get())), "right-click the reader with a blank SIM card")
                .waitMs(1_500, "the hopper must not take a card the computer hasn't used")
                .await(r -> heldBlank(r), "the blank SIM is still in the reader, the hopper is empty", 1_000)
                .send("pc", "simtool write " + IMSI_2 + " cell-red --key " + KEY_2)
                .expectOrFail("pc", "^card released: waiting to be taken or pulled out$", "written and released", "^simtool:")
                .expect("pc", PROMPT, "simtool finished")
                .await(r -> hopperHolds(r, IMSI_2), "the hopper pulled the written SIM", 5_000)
                .await(CardScenarios::keyNotOnTheWire, "the SIM as a client receives it carries no key", 1_000)
                .note("Part 3: a hopper on top, fed by a chest")
                .mutate(r -> {
                    r.player().place(Blocks.HOPPER, r.abs(HOPPER_TOP), Direction.NORTH, s -> s.getValue(HopperBlock.FACING) == Direction.DOWN);
                    r.player().place(Blocks.CHEST, r.abs(FEED), Direction.NORTH, s -> s.getBlock() instanceof ChestBlock);
                }, "place a hopper on top of the reader and a chest on that hopper")
                .send("pc", "simtool wait 30")
                .expect("pc", "^waiting for a card", "simtool waits for a card")
                .mutate(CardScenarios::dropBlankIntoFeed, "put a blank SIM card into the top chest")
                .expectOrFail("pc", "^event card_inserted sim blank=true$", "the computer hears card_inserted", "^simtool:")
                .expect("pc", PROMPT, "simtool finished")
                .send("pc", "simtool write " + IMSI_3 + " cell-red --random")
                .expectOrFail("pc", "^card released: waiting to be taken or pulled out$", "written and released", "^simtool:")
                .expect("pc", PROMPT, "simtool finished")
                .await(r -> hopperHolds(r, IMSI_3), "the hopper below pulled it too", 5_000)
                .timeLimit(60_000)
                .build();
    }

    // ------------------------------------------------------------ player actions

    static CardReaderBlockEntity reader(ScenarioRun r) {
        return r.level().getBlockEntity(r.abs(READER)) instanceof CardReaderBlockEntity be ? be : null;
    }

    /** Right-click the empty reader (opens its screen), click the Auto Eject Sim button, close. */
    static void setAutoEject(ScenarioRun r, boolean on) {
        r.player().openMenu(r.abs(READER), Direction.NORTH, true);
        boolean pressed = r.player().pressMenuButton(on ? CardReaderMenu.BUTTON_AUTO_EJECT_TRUE : CardReaderMenu.BUTTON_AUTO_EJECT_FALSE);
        r.player().entity().closeContainer();
        CardReaderBlockEntity be = reader(r);
        if (!pressed || be == null || be.autoEject() != on)
            throw new IllegalStateException("couldn't set Auto Eject Sim to " + on + " in the reader screen");
        r.say("§7  Auto Eject Sim: " + on);
    }

    static void insert(ScenarioRun r, ItemStack card) {
        List<String> said = r.player().use(card, r.abs(READER), Direction.NORTH, false);
        CardReaderBlockEntity be = reader(r);
        if (be == null || !be.hasCard()) throw new IllegalStateException("the reader took no card: the game said " + said);
        r.say("§7  card clicked into the reader (" + String.join(" ", said) + ")");
    }

    /** Open the chest, shift-click the SIM into the inventory, close, then click it into the reader. */
    static void takeFromChestAndInsert(ScenarioRun r) {
        if (r.player().openMenu(r.abs(CHEST), Direction.NORTH, false) == null) throw new IllegalStateException("the chest did not open");
        ItemStack took = ItemStack.EMPTY;
        for (int i = 0; i < 27 && took.isEmpty(); i++) {
            if (SimCardItem.isIssued(r.player().entity().containerMenu.getSlot(i).getItem())) took = r.player().shiftClick(i);
        }
        r.player().entity().closeContainer();
        if (took.isEmpty()) throw new IllegalStateException("no written SIM in the chest");
        var inv = r.player().entity().getInventory();
        ItemStack sim = ItemStack.EMPTY;
        for (int i = 0; i < inv.getContainerSize(); i++)
            if (SimCardItem.isIssued(inv.getItem(i))) { sim = inv.getItem(i).copy(); inv.setItem(i, ItemStack.EMPTY); break; }
        if (sim.isEmpty()) throw new IllegalStateException("the SIM didn't reach the inventory");
        insert(r, sim);
    }

    /** Open the feed chest and shift-click a blank SIM from the hotbar into it. */
    static void dropBlankIntoFeed(ScenarioRun r) {
        var p = r.player().entity();
        if (r.player().openMenu(r.abs(FEED), Direction.NORTH, false) == null) throw new IllegalStateException("the feed chest did not open");
        // Hotbar slot 2 (the main hand, slot 1, is what the right-click used).
        p.getInventory().setItem(1, new ItemStack(CellularContent.SIM_CARD.get()));
        int hotbar0 = p.containerMenu.slots.size() - 9;
        r.player().shiftClick(hotbar0 + 1);
        if (contents(r, FEED).isEmpty()) throw new IllegalStateException("the SIM didn't go into the feed chest");
        p.closeContainer();
        p.getInventory().clearContent();
    }

    // ------------------------------------------------------------ checks

    static List<ItemStack> contents(ScenarioRun r, BlockPos rel) {
        List<ItemStack> out = new ArrayList<>();
        if (r.level().getBlockEntity(r.abs(rel)) instanceof Container c)
            for (int i = 0; i < c.getContainerSize(); i++) if (!c.getItem(i).isEmpty()) out.add(c.getItem(i));
        return out;
    }

    static ItemStack find(List<ItemStack> items, long imsi) {
        for (ItemStack s : items) {
            SimIdentity id = SimCardItem.identity(s);
            if (id != null && id.imsi() == imsi) return s;
        }
        return null;
    }

    static String lastMatch(String screen, String re) {
        Matcher m = Pattern.compile(re, Pattern.MULTILINE).matcher(screen);
        String last = null;
        while (m.find()) last = m.group(1);
        return last;
    }

    static String chestHolds(ScenarioRun r, long imsi) {
        CardReaderBlockEntity be = reader(r);
        ItemStack s = find(contents(r, CHEST), imsi);
        if (s == null) return "chest: " + contents(r, CHEST) + ", reader " + (be == null ? "-" : be.card());
        if (be != null && be.hasCard()) return "the reader still holds " + be.card();
        SimSecret secret = SimCardItem.secret(s);
        if (secret == null || !secret.known()) return "the SIM in the chest has no key";
        if (imsi == IMSI_1) {
            String printed = lastMatch(r.screen("pc"), "^key ([0-9a-f]{32})\\s*$");
            if (printed == null) return "simtool printed no key";
            if (!printed.equals(secret.keyHex()) || printed.equals("0".repeat(32))) {
                r.fail("simtool printed key " + printed + " but the card holds another key (or an all-zero key)");
                return "failed";
            }
        }
        r.say("§7  chest: SIM " + SimCardItem.identity(s).carrier() + " " + SimCardItem.identity(s).imsiText());
        return null;
    }

    static String readerHolds(ScenarioRun r, long imsi, String carrier) {
        CardReaderBlockEntity be = reader(r);
        SimIdentity id = be == null ? null : SimCardItem.identity(be.card());
        if (id == null || id.imsi() != imsi || !id.carrier().equals(carrier)) {
            r.fail("the card in the reader changed: " + id);
            return "failed";
        }
        return null;
    }

    static String authMatchesServer(ScenarioRun r) {
        CardReaderBlockEntity be = reader(r);
        SimSecret s = be == null ? null : SimCardItem.secret(be.card());
        if (s == null || !s.known()) return "no keyed card in the reader";
        String res = lastMatch(r.screen("pc"), "^res ([0-9a-f]{32})\\s*$");
        if (res == null) return "no res on the screen";
        String want = SimAuth.respond(s.key(), HexFormat.of().parseHex(CHALLENGE)).resHex();
        if (!want.equals(res)) {
            r.fail("simtool auth printed res " + res + ", the server computes " + want);
            return "failed";
        }
        r.say("§7  RES " + res + " matches the server's HMAC of the stored key");
        return null;
    }

    static String heldBlank(ScenarioRun r) {
        CardReaderBlockEntity be = reader(r);
        if (be == null || !be.hasCard() || SimCardItem.isIssued(be.card()) || !contents(r, HOPPER_BELOW).isEmpty()) {
            r.fail("control: the unwritten card left the reader (reader " + (be == null ? "-" : be.card()) + ", hopper "
                    + contents(r, HOPPER_BELOW) + ")");
            return "failed";
        }
        return null;
    }

    static String hopperHolds(ScenarioRun r, long imsi) {
        CardReaderBlockEntity be = reader(r);
        if (find(contents(r, HOPPER_BELOW), imsi) == null)
            return "hopper: " + contents(r, HOPPER_BELOW) + ", reader " + (be == null ? "-" : be.card() + " released " + be.released());
        if (be != null && be.hasCard()) return "the reader still holds " + be.card();
        if (imsi == IMSI_3 && !contents(r, FEED).isEmpty()) return "the feed chest still has " + contents(r, FEED);
        return null;
    }

    /** Encode the written SIM as the server sends it to clients: no key, in hex or raw; it decodes to the placeholder. */
    static String keyNotOnTheWire(ScenarioRun r) {
        ItemStack s = find(contents(r, HOPPER_BELOW), IMSI_2);
        if (s == null) return "no SIM 2";
        RegistryFriendlyByteBuf buf = new RegistryFriendlyByteBuf(Unpooled.buffer(), r.level().registryAccess());
        ItemStack.OPTIONAL_STREAM_CODEC.encode(buf, s);
        byte[] wire = ByteBufUtil.getBytes(buf);
        String text = new String(wire, StandardCharsets.ISO_8859_1).toLowerCase(Locale.ROOT);
        boolean raw = ByteBufUtil.indexOf(Unpooled.wrappedBuffer(HexFormat.of().parseHex(KEY_2)), Unpooled.wrappedBuffer(wire)) >= 0;
        if (text.contains(KEY_2) || raw) {
            r.fail("the SIM's network encoding contains its key");
            return "failed";
        }
        ItemStack client = ItemStack.OPTIONAL_STREAM_CODEC.decode(new RegistryFriendlyByteBuf(Unpooled.wrappedBuffer(wire), r.level().registryAccess()));
        SimSecret cs = SimCardItem.secret(client);
        if (!client.has(CellularContent.SIM_SECRET.get()) || cs == null || cs.known()) {
            r.fail("a client's copy should hold the hidden placeholder, has " + cs);
            return "failed";
        }
        if (!SimCardItem.identity(client).equals(SimCardItem.identity(s))) {
            r.fail("a client's copy lost the identity");
            return "failed";
        }
        r.say("§7  network encoding: " + wire.length + " bytes, identity " + SimCardItem.identity(client).imsiText() + ", no key");
        return null;
    }
}
//?}
