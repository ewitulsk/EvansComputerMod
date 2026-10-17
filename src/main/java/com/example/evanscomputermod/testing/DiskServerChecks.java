package com.example.evanscomputermod.testing;

//? if <=1.21.1 {
import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.disk.DiskContent;
import com.example.evanscomputermod.disk.DiskDriveBlock;
import com.example.evanscomputermod.disk.DiskDriveBlockEntity;
import com.mojang.brigadier.CommandDispatcher;
import net.minecraft.commands.CommandSourceStack;
import net.minecraft.commands.Commands;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.server.MinecraftServer;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.server.level.ServerPlayer;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.level.GameRules;
import net.minecraft.world.level.GameType;
import net.minecraft.world.level.block.Blocks;
import net.neoforged.neoforge.event.tick.ServerTickEvent;

/**
 * Disk render suite, server half ({@code -ClientChecks -ClientSuite disks}): two Disk
 * Drives two blocks east of the player, their fronts towards the player, the north one
 * empty and the south one with a floppy in (the {@code disk=true} model); a floppy and a
 * drive in the player's hotbar. The hidden client ({@code DiskClientChecks}) asks for each
 * case with {@code /ecmvisual disks <case>}; the server checks the state the picture should
 * show and logs {@code ECM_VISUAL_SERVER_PASS <case>}.
 */
public final class DiskServerChecks {
    public static final BlockPos ORIGIN = new BlockPos(0, 170, 0);
    public static final BlockPos EMPTY = ORIGIN.offset(2, 0, -1), FULL = ORIGIN.offset(2, 0, 1);

    private static boolean built, failed;
    private static int finishTicks = -1;

    private DiskServerChecks() {}

    public static void register(CommandDispatcher<CommandSourceStack> d) {
        if (!RadioVisualLayout.active("disks")) return;
        d.register(Commands.literal("ecmvisual").then(Commands.literal("disks")
                .then(Commands.literal("blocks").executes(c -> look(c.getSource(), -90f, 15f, "disks_blocks", DiskServerChecks::blocksReady)))
                .then(Commands.literal("inventory").executes(c -> look(c.getSource(), -90f, 0f, "disks_inventory", s -> {
                    var inv = s.getPlayerOrException().getInventory();
                    return inv.getItem(0).is(DiskContent.FLOPPY_DISK.get()) && inv.getItem(1).is(DiskContent.DISK_DRIVE_ITEM.get()) ? null
                            : "hotbar: " + inv.getItem(0) + ", " + inv.getItem(1);
                })))
                .then(Commands.literal("finish").executes(c -> {
                    EvansComputerMod.LOGGER.info("ECM_VISUAL_SERVER_PASS disks_final");
                    finishTicks = 60;
                    return 1;
                }))));
    }

    private interface Check {
        String problem(CommandSourceStack s) throws Exception;
    }

    private static int look(CommandSourceStack s, float yaw, float pitch, String name, Check check) {
        try {
            ServerPlayer p = s.getPlayerOrException();
            p.teleportTo(s.getLevel(), ORIGIN.getX() + 0.5, ORIGIN.getY(), ORIGIN.getZ() + 0.5, yaw, pitch);
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
        if (!(level.getBlockEntity(EMPTY) instanceof DiskDriveBlockEntity e) || e.hasDisk() || level.getBlockState(EMPTY).getValue(DiskDriveBlock.DISK))
            return "the north drive should be empty";
        if (!(level.getBlockEntity(FULL) instanceof DiskDriveBlockEntity f) || !f.hasDisk() || !level.getBlockState(FULL).getValue(DiskDriveBlock.DISK))
            return "the south drive should hold a floppy";
        return null;
    }

    private static void fail(MinecraftServer server, String why) {
        failed = true;
        EvansComputerMod.LOGGER.error("ECM_VISUAL_SERVER_FAIL {}", why);
        server.halt(false);
    }

    public static void tick(ServerTickEvent.Post event) {
        if (!RadioVisualLayout.active("disks") || failed) return;
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
                EvansComputerMod.LOGGER.info("ECM_VISUAL_SERVER_PASS disks_fixture");
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
        for (int x = -4; x <= 6; x++)
            for (int z = -5; z <= 5; z++) {
                level.setBlock(ORIGIN.offset(x, -1, z), Blocks.SMOOTH_STONE.defaultBlockState(), 2);
                for (int y = 0; y <= 4; y++) level.setBlock(ORIGIN.offset(x, y, z), Blocks.AIR.defaultBlockState(), 2);
            }
        for (BlockPos at : new BlockPos[] {EMPTY, FULL})
            level.setBlock(at, DiskContent.DISK_DRIVE.get().defaultBlockState().setValue(DiskDriveBlock.FACING, Direction.WEST), 3);
        if (level.getBlockEntity(FULL) instanceof DiskDriveBlockEntity f)
            f.automation().insertItem(0, new ItemStack(DiskContent.FLOPPY_DISK.get()), false);

        player.setGameMode(GameType.CREATIVE);
        player.getAbilities().flying = false;
        player.onUpdateAbilities();
        player.getInventory().clearContent();
        player.getInventory().setItem(0, new ItemStack(DiskContent.FLOPPY_DISK.get()));
        player.getInventory().setItem(1, new ItemStack(DiskContent.DISK_DRIVE_ITEM.get()));
        player.getInventory().selected = 0;
        player.teleportTo(level, ORIGIN.getX() + 0.5, ORIGIN.getY(), ORIGIN.getZ() + 0.5, -90f, 15f);
    }
}
//?}
