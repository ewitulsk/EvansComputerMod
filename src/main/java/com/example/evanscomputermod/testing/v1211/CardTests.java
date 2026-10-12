package com.example.evanscomputermod.testing.v1211;

//? if <=1.21.1 {
import com.example.evanscomputermod.testing.scenario.RadioScenarios;
import net.minecraft.gametest.framework.GameTest;
import net.minecraft.gametest.framework.GameTestHelper;
import net.neoforged.neoforge.gametest.GameTestHolder;
import net.neoforged.neoforge.gametest.PrefixGameTestTemplate;

/**
 * Smart cards, namespace {@code ecm_cards} (1.21.1): the {@code card_writer} scenario
 * (CardScenarios) as a test. Structure {@code gametest_cards} (12x8x10).
 */
@GameTestHolder(CardTests.NS)
@PrefixGameTestTemplate(false)
public final class CardTests {
    static final String NS = "ecm_cards";
    private static final String STRUCTURE = "gametest_cards";

    /**
     * A player writes SIMs with simtool: Auto Eject Sim into a chest, a hopper pulling with it
     * off, a hopper feeding; controls: a written SIM can't be rewritten, an unwritten card is
     * held, the key is not in the client encoding.
     */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".card_writer")
    public static void card_writer(GameTestHelper h) {
        TestDriver.scenario(h, NS, RadioScenarios.ALL.get("card_writer"), true);
    }

    /**
     * A creative client only ever has the hidden placeholder for a SIM's key; when it moves the
     * card (empty the old slot, fill the new one with its copy), SimSecretGuard puts the server's
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

        com.example.evanscomputermod.card.SimSecretGuard.beforeCreativeSet(player, 36, net.minecraft.world.item.ItemStack.EMPTY);
        player.getInventory().setItem(0, net.minecraft.world.item.ItemStack.EMPTY);
        com.example.evanscomputermod.card.SimSecretGuard.beforeCreativeSet(player, 37, clientCopy);
        var restored = com.example.evanscomputermod.radio.cellular.SimCardItem.secret(clientCopy);
        h.assertTrue(restored != null && restored.known() && restored.keyHex().equals("00112233445566778899aabbccddeeff"),
                "the moved SIM got its key back: " + restored);

        net.minecraft.world.item.ItemStack forged = clientCopy.copy();
        forged.set(com.example.evanscomputermod.radio.cellular.CellularContent.SIM_CARD_DATA.get(),
                new com.example.evanscomputermod.card.SimIdentity("cell-red", 310_150_000_000_078L));
        forged.set(com.example.evanscomputermod.radio.cellular.CellularContent.SIM_SECRET.get(), com.example.evanscomputermod.card.SimSecret.HIDDEN);
        com.example.evanscomputermod.card.SimSecretGuard.beforeCreativeSet(player, 38, forged);
        h.assertTrue(!forged.has(com.example.evanscomputermod.radio.cellular.CellularContent.SIM_SECRET.get()),
                "a placeholder SIM nobody held stays keyless");
        com.example.evanscomputermod.EvansComputerMod.LOGGER.info("ECM_CARDS_TEST_PASS creative_slot_keeps_key");
        h.succeed();
    }

    private CardTests() {}
}
//?}
