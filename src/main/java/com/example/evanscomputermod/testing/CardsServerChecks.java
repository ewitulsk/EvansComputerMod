package com.example.evanscomputermod.testing;

//? if <=1.21.1 {
import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.card.CardContent;
import com.example.evanscomputermod.card.CardItem;
import com.example.evanscomputermod.card.CardReaderBlock;
import com.example.evanscomputermod.card.CardReaderBlockEntity;
import com.example.evanscomputermod.card.nfc.NfcReaderBlock;
import com.example.evanscomputermod.card.nfc.NfcReaderBlockEntity;
import com.example.evanscomputermod.card.printer.CardPrinterBlock;
import com.example.evanscomputermod.card.printer.CardPrinterBlockEntity;
import com.example.evanscomputermod.radio.cellular.CellularContent;
import com.mojang.brigadier.CommandDispatcher;
import net.minecraft.commands.CommandSourceStack;
import net.minecraft.commands.Commands;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.server.MinecraftServer;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.server.level.ServerPlayer;
import net.minecraft.world.entity.decoration.ItemFrame;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.item.Items;
import net.minecraft.world.level.GameRules;
import net.minecraft.world.level.GameType;
import net.minecraft.world.level.block.Blocks;
import net.neoforged.neoforge.event.tick.ServerTickEvent;

/**
 * Cards render suite, server half ({@code -ClientChecks -ClientSuite cards}): a Card
 * Reader/Writer (with a printed bank card in it), a Card Printer (dye and a card loaded) and
 * an NFC Reader (field on) in a row two blocks east of the player, four printed cards in item
 * frames on a wall, and a printed bank card in the player's hand. The hidden client
 * ({@code CardsClientChecks}) asks for each case with {@code /ecmvisual cards <case>}; the
 * server checks the state the picture should show and logs {@code ECM_VISUAL_SERVER_PASS <case>}.
 */
public final class CardsServerChecks {
    public static final BlockPos ORIGIN = new BlockPos(0, 170, 0);
    public static final BlockPos READER = ORIGIN.offset(2, 0, -2), PRINTER = ORIGIN.offset(2, 0, 0), NFC = ORIGIN.offset(2, 0, 2);
    /** Item frames hang on the west face of a wall at x+6, at z -2..1. */
    public static final int FRAME_X = 5, WALL_X = 6;

    private static boolean built, failed;
    private static int finishTicks = -1;

    private CardsServerChecks() {}

    public static void register(CommandDispatcher<CommandSourceStack> d) {
        if (!RadioVisualLayout.active("cards")) return;
        d.register(Commands.literal("ecmvisual").then(Commands.literal("cards")
                .then(Commands.literal("blocks").executes(c -> look(c.getSource(), 0.5, 0.5, -90f, 22f, "cards_blocks", CardsServerChecks::blocksReady)))
                .then(Commands.literal("hand").executes(c -> look(c.getSource(), 0.5, 0.5, -90f, 10f, "cards_hand", s -> {
                    ItemStack h = s.getPlayerOrException().getMainHandItem();
                    return CardItem.kindOf(h) == com.example.evanscomputermod.card.CardKind.BANK && CardItem.faces(h) != null ? null : "the main hand holds " + h;
                })))
                .then(Commands.literal("frames").executes(c -> look(c.getSource(), 2.6, -0.5, -90f, 0f, "cards_frames", s -> {
                    int n = s.getLevel().getEntitiesOfClass(ItemFrame.class, net.minecraft.world.phys.AABB.encapsulatingFullBlocks(ORIGIN.offset(FRAME_X, 0, -2), ORIGIN.offset(FRAME_X, 0, 1)))
                            .stream().filter(f -> CardItem.faces(f.getItem()) != null).toList().size();
                    return n == 4 ? null : n + " framed printed cards";
                })))
                .then(Commands.literal("printer").executes(c -> look(c.getSource(), 1.5, 0.5, -90f, 35f, "cards_printer_screen", s -> {
                    if (!(s.getLevel().getBlockEntity(PRINTER) instanceof CardPrinterBlockEntity be)) return "no printer";
                    return !be.cardIn().isEmpty() && be.dyes().size() >= 3 ? null : "printer not loaded: " + be.dyes();
                })))
                .then(Commands.literal("inventory").executes(c -> look(c.getSource(), 0.5, 0.5, -90f, 0f, "cards_inventory", s -> {
                    var inv = s.getPlayerOrException().getInventory();
                    return CardItem.kindOf(inv.getItem(0)) != null && CardItem.kindOf(inv.getItem(1)) != null ? null : "hotbar: " + inv.getItem(0) + ", " + inv.getItem(1);
                })))
                .then(Commands.literal("finish").executes(c -> {
                    EvansComputerMod.LOGGER.info("ECM_VISUAL_SERVER_PASS cards_final");
                    finishTicks = 60;
                    return 1;
                }))));
    }

    private interface Check {
        String problem(CommandSourceStack s) throws Exception;
    }

    /** Put the player at ORIGIN + (dx, 0, dz) looking (yaw, pitch); pass the case if {@code check} finds no problem. */
    private static int look(CommandSourceStack s, double dx, double dz, float yaw, float pitch, String name, Check check) {
        try {
            ServerPlayer p = s.getPlayerOrException();
            p.teleportTo(s.getLevel(), ORIGIN.getX() + dx, ORIGIN.getY(), ORIGIN.getZ() + dz, yaw, pitch);
            String problem = check.problem(s);
            if (problem != null) {
                fail(s.getServer(), name + ": " + problem);
                return 0;
            }
            EvansComputerMod.LOGGER.info("ECM_VISUAL_SERVER_PASS {}", name);
            return 1;
        } catch (Exception e) {
            fail(s.getServer(), name + ": " + e);
            return 0;
        }
    }

    private static String blocksReady(CommandSourceStack s) {
        ServerLevel level = s.getLevel();
        if (!(level.getBlockEntity(READER) instanceof CardReaderBlockEntity r) || !r.hasCard()) return "the reader holds no card";
        if (!(level.getBlockEntity(NFC) instanceof NfcReaderBlockEntity n) || !level.getBlockState(NFC).getValue(NfcReaderBlock.FIELD))
            return "the NFC reader's field is off";
        if (!(level.getBlockEntity(PRINTER) instanceof CardPrinterBlockEntity)) return "no printer";
        return null;
    }

    private static void fail(MinecraftServer server, String why) {
        failed = true;
        EvansComputerMod.LOGGER.error("ECM_VISUAL_SERVER_FAIL {}", why);
        server.halt(false);
    }

    public static void tick(ServerTickEvent.Post event) {
        if (!RadioVisualLayout.active("cards") || failed) return;
        MinecraftServer server = event.getServer();
        if (finishTicks >= 0) {
            if (--finishTicks == 0) server.halt(false);
            return;
        }
        if (server.getPlayerList().getPlayers().isEmpty()) return;
        ServerPlayer player = server.getPlayerList().getPlayers().get(0);
        try {
            if (!built) {
                build(server, server.overworld(), player);
                built = true;
                EvansComputerMod.LOGGER.info("ECM_VISUAL_SERVER_PASS cards_fixture");
            }
        } catch (Throwable t) {
            failed = true;
            EvansComputerMod.LOGGER.error("ECM_VISUAL_SERVER_FAIL", t);
            server.halt(false);
        }
    }

    private static void build(MinecraftServer server, ServerLevel level, ServerPlayer player) {
        server.getPlayerList().op(player.getGameProfile());
        level.getGameRules().getRule(GameRules.RULE_DAYLIGHT).set(false, server);
        level.getGameRules().getRule(GameRules.RULE_WEATHER_CYCLE).set(false, server);
        level.getGameRules().getRule(GameRules.RULE_DOMOBSPAWNING).set(false, server);
        level.setDayTime(6000);
        for (int x = -4; x <= 8; x++)
            for (int z = -6; z <= 6; z++) {
                level.setBlock(ORIGIN.offset(x, -1, z), Blocks.SMOOTH_STONE.defaultBlockState(), 2);
                for (int y = 0; y <= 5; y++) level.setBlock(ORIGIN.offset(x, y, z), Blocks.AIR.defaultBlockState(), 2);
            }
        // The wall the item frames hang on, facing the player.
        for (int z = -4; z <= 4; z++)
            for (int y = 0; y <= 2; y++) level.setBlock(ORIGIN.offset(WALL_X, y, z), Blocks.SMOOTH_STONE.defaultBlockState(), 2);
        var kinds = new ItemStack[] {new ItemStack(CardContent.BANK_CARD.get()), new ItemStack(CardContent.NFC_CARD.get()),
                new ItemStack(CardContent.HYBRID_CARD.get()), new ItemStack(CellularContent.SIM_CARD.get())};
        for (int i = 0; i < kinds.length; i++) {
            var frame = new ItemFrame(level, ORIGIN.offset(FRAME_X, 0, -2 + i), Direction.WEST);
            frame.setItem(RadioServerChecks.demoCard(kinds[i]));
            frame.setInvulnerable(true);
            level.addFreshEntity(frame);
        }
        // The three blocks, their fronts (facing) towards the player (west).
        level.setBlock(READER, CardContent.CARD_READER.get().defaultBlockState().setValue(CardReaderBlock.FACING, Direction.WEST), 3);
        level.setBlock(PRINTER, CardContent.CARD_PRINTER.get().defaultBlockState().setValue(CardPrinterBlock.FACING, Direction.WEST), 3);
        level.setBlock(NFC, CardContent.NFC_READER.get().defaultBlockState().setValue(NfcReaderBlock.FACING, Direction.WEST), 3);
        if (level.getBlockEntity(READER) instanceof CardReaderBlockEntity r)
            r.automation().insertItem(0, RadioServerChecks.demoCard(new ItemStack(CardContent.BANK_CARD.get())), false);
        if (level.getBlockEntity(PRINTER) instanceof CardPrinterBlockEntity p) {
            var h = p.automation(Direction.NORTH);
            h.insertItem(CardPrinterBlockEntity.SLOT_IN, new ItemStack(CardContent.BANK_CARD.get()), false);
            for (var dye : new ItemStack[] {new ItemStack(Items.WHITE_DYE, 3), new ItemStack(Items.RED_DYE, 2), new ItemStack(Items.BLUE_DYE, 4), new ItemStack(Items.BLACK_DYE, 1)})
                for (int s = CardPrinterBlockEntity.SLOT_DYE0; s < CardPrinterBlockEntity.SLOTS && !dye.isEmpty(); s++) dye = h.insertItem(s, dye, false);
        }
        if (level.getBlockEntity(NFC) instanceof NfcReaderBlockEntity n) n.peripheral().setField(true);

        player.setGameMode(GameType.CREATIVE);
        player.getAbilities().flying = false;
        player.onUpdateAbilities();
        player.getInventory().clearContent();
        player.getInventory().setItem(0, RadioServerChecks.demoCard(new ItemStack(CardContent.BANK_CARD.get())));
        player.getInventory().setItem(1, RadioServerChecks.demoCard(new ItemStack(CardContent.NFC_CARD.get())));
        player.getInventory().setItem(2, new ItemStack(CardContent.HYBRID_CARD.get()));
        player.getInventory().selected = 0;
        player.teleportTo(level, ORIGIN.getX() + 0.5, ORIGIN.getY(), ORIGIN.getZ() + 0.5, -90f, 22f);
    }
}
//?}
