package com.example.evanscomputermod.testing.v1211;

//? if <=1.21.1 {
import com.example.evanscomputermod.testing.scenario.RadioScenarios;
import net.minecraft.gametest.framework.GameTest;
import net.minecraft.gametest.framework.GameTestHelper;
import net.neoforged.neoforge.gametest.GameTestHolder;
import net.neoforged.neoforge.gametest.PrefixGameTestTemplate;

/**
 * Cards, namespace {@code ecm_cards} (1.21.1): the {@code card_writer} scenario (CardScenarios) and
 * the card system's scenarios (CardSystemScenarios: card_print, card_bank, card_stripe, card_nfc,
 * card_reader_module) as tests, plus the creative-slot secret guard and the NFC tracker's budget.
 * Structure {@code gametest_cards} (16x8x12).
 */
@GameTestHolder(CardTests.NS)
@PrefixGameTestTemplate(false)
public final class CardTests {
    static final String NS = "ecm_cards";
    private static final String STRUCTURE = "gametest_cards";

    /**
     * A player writes SIMs with cardtool sim: Auto Eject Sim into a chest, a hopper pulling with it
     * off, a hopper feeding; controls: a written SIM can't be rewritten, an unwritten card is
     * held, the key is not in the client encoding.
     */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".card_writer")
    public static void card_writer(GameTestHelper h) {
        TestDriver.scenario(h, NS, RadioScenarios.ALL.get("card_writer"), true);
    }

    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".card_print")
    public static void card_print(GameTestHelper h) {
        TestDriver.scenario(h, NS, RadioScenarios.ALL.get("card_print"), true);
    }

    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".card_bank")
    public static void card_bank(GameTestHelper h) {
        TestDriver.scenario(h, NS, RadioScenarios.ALL.get("card_bank"), true);
    }

    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".card_stripe")
    public static void card_stripe(GameTestHelper h) {
        TestDriver.scenario(h, NS, RadioScenarios.ALL.get("card_stripe"), true);
    }

    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".card_nfc")
    public static void card_nfc(GameTestHelper h) {
        TestDriver.scenario(h, NS, RadioScenarios.ALL.get("card_nfc"), true);
    }

    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".card_reader_module")
    public static void card_reader_module(GameTestHelper h) {
        TestDriver.scenario(h, NS, RadioScenarios.ALL.get("card_reader_module"), true);
    }

    /**
     * The NFC tracker's budget: twenty chests with cards inside one reader's field cost under
     * 1 ms a tick to scan (a cold first measurement; typically ~0.2 ms), and every card gets an endpoint; with the field off nothing is tracked.
     */
    @GameTest(template = STRUCTURE, batch = NS + ".tracker_budget")
    public static void tracker_budget(GameTestHelper h) {
        var level = h.getLevel();
        net.minecraft.core.BlockPos reader = h.absolutePos(new net.minecraft.core.BlockPos(7, 2, 5));
        level.setBlock(reader.below(), net.minecraft.world.level.block.Blocks.STONE.defaultBlockState(), 3);
        level.setBlock(reader, com.example.evanscomputermod.card.CardContent.NFC_READER.get().defaultBlockState(), 3);
        int chests = 0;
        for (int dx = -2; dx <= 2; dx++)
            for (int dz = -2; dz <= 2; dz++) {
                if (dx == 0 && dz == 0 || chests >= 20) continue;
                net.minecraft.core.BlockPos p = reader.offset(dx, 0, dz);
                level.setBlock(p, net.minecraft.world.level.block.Blocks.CHEST.defaultBlockState(), 3);
                if (level.getBlockEntity(p) instanceof net.minecraft.world.level.block.entity.ChestBlockEntity c) {
                    c.setItem(0, new net.minecraft.world.item.ItemStack(com.example.evanscomputermod.card.CardContent.NFC_CARD.get()));
                    chests++;
                }
            }
        int want = chests;
        h.runAfterDelay(5, () -> {
            h.assertTrue(com.example.evanscomputermod.card.nfc.CardEndpointTracker.tracked() == 0, "no field: nothing tracked, got "
                    + com.example.evanscomputermod.card.nfc.CardEndpointTracker.tracked());
            var be = (com.example.evanscomputermod.card.nfc.NfcReaderBlockEntity) level.getBlockEntity(reader);
            try {
                be.peripheral().setField(true);
            } catch (RuntimeException e) {
                throw new net.minecraft.gametest.framework.GameTestAssertException(e.getMessage());
            }
        });
        h.runAfterDelay(30, () -> {
            int tracked = com.example.evanscomputermod.card.nfc.CardEndpointTracker.tracked();
            long ns = com.example.evanscomputermod.card.nfc.CardEndpointTracker.lastScanNanos();
            h.assertTrue(tracked == want, "every card in the field has an endpoint: " + tracked + " of " + want);
            h.assertTrue(ns < 1_000_000, "the scan took " + ns / 1000 + " us (budget 1 ms)");
            com.example.evanscomputermod.EvansComputerMod.LOGGER.info("ECM_CARDS_TEST_PASS tracker_budget {} endpoints, scan {} us", tracked, ns / 1000);
            h.succeed();
        });
    }

    /**
     * A creative client only ever has the hidden placeholder for a SIM's key; when it moves the
     * card (empty the old slot, fill the new one with its copy), CardSecretGuard puts the server's
     * key back. A placeholder SIM nobody held is stored without a key. (NeoForge's FakePlayer
     * connection ignores creative-slot packets, so this drives the guard the mixin calls.)
     */
    @GameTest(template = STRUCTURE, batch = NS + ".creative_slot")
    public static void creative_slot_keeps_key(GameTestHelper h) {
        var player = new net.neoforged.neoforge.common.util.FakePlayer(h.getLevel(),
                new com.mojang.authlib.GameProfile(java.util.UUID.nameUUIDFromBytes("ecm-cards".getBytes()), "CardTester"));
        player.setGameMode(net.minecraft.world.level.GameType.CREATIVE);
        var registries = h.getLevel().registryAccess();
        net.minecraft.world.item.ItemStack sim = new net.minecraft.world.item.ItemStack(
                com.example.evanscomputermod.radio.cellular.CellularContent.SIM_CARD.get());
        try {
            com.example.evanscomputermod.radio.cellular.SimCardItem.card(sim).write(310_150_000_000_077L, "cell-red",
                    java.util.HexFormat.of().parseHex("00112233445566778899aabbccddeeff"));
        } catch (com.example.evanscomputermod.card.SmartSim.CardException e) {
            throw new net.minecraft.gametest.framework.GameTestAssertException(e.getMessage());
        }
        player.getInventory().setItem(0, sim);                     // inventoryMenu slot 36
        var buf = new net.minecraft.network.RegistryFriendlyByteBuf(io.netty.buffer.Unpooled.buffer(), registries);
        net.minecraft.world.item.ItemStack.OPTIONAL_STREAM_CODEC.encode(buf, sim);
        net.minecraft.world.item.ItemStack clientCopy = net.minecraft.world.item.ItemStack.OPTIONAL_STREAM_CODEC.decode(buf);
        h.assertTrue(!com.example.evanscomputermod.radio.cellular.SimCardItem.secret(clientCopy).known(), "client copy has no key");

        com.example.evanscomputermod.card.CardSecretGuard.beforeCreativeSet(player, 36, net.minecraft.world.item.ItemStack.EMPTY);
        player.getInventory().setItem(0, net.minecraft.world.item.ItemStack.EMPTY);
        com.example.evanscomputermod.card.CardSecretGuard.beforeCreativeSet(player, 37, clientCopy);
        var restored = com.example.evanscomputermod.radio.cellular.SimCardItem.secret(clientCopy);
        h.assertTrue(restored != null && restored.known() && restored.keyHex().equals("00112233445566778899aabbccddeeff"),
                "the moved SIM got its key back: " + restored);

        net.minecraft.world.item.ItemStack forged = clientCopy.copy();
        forged.set(com.example.evanscomputermod.radio.cellular.CellularContent.SIM_CARD_DATA.get(),
                new com.example.evanscomputermod.card.SimIdentity("cell-red", 310_150_000_000_078L));
        forged.set(com.example.evanscomputermod.radio.cellular.CellularContent.SIM_SECRET.get(), com.example.evanscomputermod.card.SimSecret.HIDDEN);
        com.example.evanscomputermod.card.CardSecretGuard.beforeCreativeSet(player, 38, forged);
        h.assertTrue(!forged.has(com.example.evanscomputermod.radio.cellular.CellularContent.SIM_SECRET.get()),
                "a placeholder SIM nobody held stays keyless");
        com.example.evanscomputermod.EvansComputerMod.LOGGER.info("ECM_CARDS_TEST_PASS creative_slot_keeps_key");
        h.succeed();
    }

    private CardTests() {}
}
//?}
