package com.example.evanscomputermod.testing;

//? if <=1.21.1 {
import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.computer.peripheral.PeripheralEventBus;
import com.example.evanscomputermod.computer.peripheral.PeripheralValues;
import com.example.evanscomputermod.phone.ChargingDockBlock;
import com.example.evanscomputermod.phone.ChargingDockBlockEntity;
import com.example.evanscomputermod.phone.PhoneContent;
import com.example.evanscomputermod.phone.PhoneHost;
import com.example.evanscomputermod.phone.PhoneItemData;
import com.example.evanscomputermod.phone.PhoneServer;
import com.example.evanscomputermod.phone.PhoneState;
import com.example.evanscomputermod.testing.scenario.ScenarioRun;
import com.mojang.brigadier.CommandDispatcher;
import net.minecraft.commands.CommandSourceStack;
import net.minecraft.commands.Commands;
import net.minecraft.core.BlockPos;
import net.minecraft.server.MinecraftServer;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.server.level.ServerPlayer;
import net.minecraft.world.InteractionHand;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.level.GameRules;
import net.minecraft.world.level.GameType;
import net.minecraft.world.level.block.Blocks;
import net.neoforged.neoforge.event.tick.ServerTickEvent;

import java.util.ArrayList;
import java.util.List;
import java.util.UUID;

/**
 * Phone render suite, server half ({@code -Decm.clientChecks.suite=phone}): the
 * joining player gets an ePhone in hotbar slot 1 (held) on a stone platform;
 * once it has booted, {@code echo PHONE_RENDER_OK} is typed on it. The hidden
 * client ({@code PhoneClientChecks}) photographs it in the main hand, asks for
 * {@code ecmvisual phone offhand} (the server moves it to the off hand and
 * reports what it had in the main hand), photographs it there, then walks
 * forward with hotkey slot 1 bound to W: the server reports the
 * {@code phone_hotkey} events it got while the client reports how far it walked.
 */
public final class PhoneServerChecks {
    public static final BlockPos ORIGIN = new BlockPos(0, 170, 0);

    private static boolean built, typed, failed, offhandDone, hotkeyDone;
    private static int finishTicks = -1;
    private static UUID phone;
    private static BlockPos dockPos;
    private static boolean dockReported;
    private static PeripheralEventBus.Subscriber events;
    private static final List<String> heard = new ArrayList<>();

    private PhoneServerChecks() {}

    public static void register(CommandDispatcher<CommandSourceStack> d) {
        if (!RadioVisualLayout.active("phone")) return;
        d.register(Commands.literal("ecmvisual").then(Commands.literal("phone")
                .then(Commands.literal("offhand").executes(c -> {
                    ServerPlayer p = c.getSource().getPlayerOrException();
                    PhoneHost h = host(c.getSource().getServer());
                    if (h == null || h.state() != PhoneState.AWAKE || !p.getUUID().equals(h.holderId())
                            || !phone.equals(PhoneItemData.id(p.getMainHandItem()))) {
                        fail(c.getSource().getServer(), "main hand: phone not awake in the player's main hand: " + h);
                        return 0;
                    }
                    EvansComputerMod.LOGGER.info("ECM_VISUAL_SERVER_PASS phone_main_hand state={} display={}x{}", h.state(),
                            h.display().getWidth(), h.display().getHeight());
                    ItemStack s = p.getInventory().getItem(0);
                    p.getInventory().setItem(0, ItemStack.EMPTY);
                    p.setItemInHand(InteractionHand.OFF_HAND, s);
                    p.getInventory().selected = 1;
                    offhandDone = false;
                    return 1;
                }))
                .then(Commands.literal("dock").executes(c -> {
                    // A Charging Dock two blocks in front of the player, facing it, with an 80 % phone docked.
                    ServerPlayer p = c.getSource().getPlayerOrException();
                    ServerLevel level = c.getSource().getLevel();
                    BlockPos dock = ORIGIN.offset(2, 0, 0);
                    level.setBlock(dock, PhoneContent.CHARGING_DOCK.get().defaultBlockState()
                            .setValue(ChargingDockBlock.FACING, net.minecraft.core.Direction.WEST), 3);
                    ItemStack docked = new ItemStack(PhoneContent.EPHONE.get());
                    PhoneItemData.ensureId(docked);
                    PhoneItemData.setBatteryPercent(docked, 80);
                    if (level.getBlockEntity(dock) instanceof ChargingDockBlockEntity be) be.setPhone(docked);
                    p.teleportTo(level, ORIGIN.getX() + 0.5, ORIGIN.getY(), ORIGIN.getZ() + 0.5, -90f, 35f);
                    dockPos = dock;
                    return 1;
                }))
                .then(Commands.literal("finish").executes(c -> {
                    EvansComputerMod.LOGGER.info("ECM_VISUAL_SERVER_PASS phone_final");
                    finishTicks = 60;
                    return 1;
                }))));
    }

    private static PhoneHost host(MinecraftServer server) {
        return phone == null ? null : PhoneServer.find(server, phone);
    }

    private static void fail(MinecraftServer server, String why) {
        failed = true;
        EvansComputerMod.LOGGER.error("ECM_VISUAL_SERVER_FAIL {}", why);
        server.halt(false);
    }

    public static void tick(ServerTickEvent.Post event) {
        if (!RadioVisualLayout.active("phone") || failed) return;
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
                return;
            }
            PhoneHost h = host(server);
            if (h == null) return;
            if (!typed) {
                if (h.instance() == null || !ScenarioRun.screen(h.display()).contains("Welcome to Terminal OS")
                        || h.display().getWidth() != 30) return;
                h.instance().handleKeyInput("echo PHONE_RENDER_OK\n");
                events = h.instance().getPeripheralEvents().subscribe();
                typed = true;
                EvansComputerMod.LOGGER.info("ECM_VISUAL_SERVER_PASS phone_fixture phone={} console={}x{}", phone,
                        h.display().getWidth(), h.display().getHeight());
                return;
            }
            if (!offhandDone && phone.equals(PhoneItemData.id(player.getOffhandItem())) && h.state() == PhoneState.AWAKE
                    && player.getUUID().equals(h.holderId())) {
                offhandDone = true;
                EvansComputerMod.LOGGER.info("ECM_VISUAL_SERVER_PASS phone_off_hand state={}", h.state());
            }
            if (dockPos != null && !dockReported && server.overworld().getBlockState(dockPos).getValue(ChargingDockBlock.ACTIVE)
                    && server.overworld().getBlockEntity(dockPos) instanceof ChargingDockBlockEntity be && !be.phoneStack().isEmpty()) {
                dockReported = true;
                EvansComputerMod.LOGGER.info("ECM_VISUAL_SERVER_PASS phone_dock active=true battery={}",
                        PhoneItemData.batteryPercent(be.phoneStack()));
            }
            if (!hotkeyDone && events != null) {
                PeripheralEventBus.Event e;
                while ((e = PeripheralEventBus.await(events, "phone_hotkey", 0)) != null) heard.add(String.valueOf(PeripheralValues.decode(e.encoded())));
                if (heard.contains("[phone_hotkey, phone, 1, true]") && heard.contains("[phone_hotkey, phone, 1, false]")) {
                    hotkeyDone = true;
                    EvansComputerMod.LOGGER.info("ECM_VISUAL_SERVER_PASS phone_hotkey events={}", heard);
                }
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
        for (int x = -6; x <= 30; x++)
            for (int z = -6; z <= 6; z++) {
                level.setBlock(ORIGIN.offset(x, -1, z), Blocks.SMOOTH_STONE.defaultBlockState(), 2);
                for (int y = 0; y <= 5; y++) level.setBlock(ORIGIN.offset(x, y, z), Blocks.AIR.defaultBlockState(), 2);
            }
        // Something to look at past the phone.
        for (int y = 0; y <= 3; y++)
            for (int z = -3; z <= 3; z++) level.setBlock(ORIGIN.offset(30, y, z), Blocks.BRICKS.defaultBlockState(), 2);
        player.setGameMode(GameType.CREATIVE);
        player.getAbilities().flying = false;
        player.onUpdateAbilities();
        player.teleportTo(level, ORIGIN.getX() + 0.5, ORIGIN.getY(), ORIGIN.getZ() + 0.5, -90f, 0f);
        ItemStack s = new ItemStack(PhoneContent.EPHONE.get());
        phone = PhoneItemData.ensureId(s);
        player.getInventory().setItem(0, s);
        player.getInventory().selected = 0;
        player.connection.send(new net.minecraft.network.protocol.game.ClientboundSetCarriedItemPacket(0));
    }
}
//?}
