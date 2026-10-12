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
 * joining player gets an ePhone in hotbar slot 1 (held) on a stone platform; it
 * boots into the phone shell (P5b). The hidden client ({@code PhoneClientChecks})
 * photographs the shell in the main hand, asks for {@code ecmvisual phone offhand}
 * (the server moves it to the off hand and reports what it had in the main hand),
 * photographs it there, opens the phone and taps Notes (the server reports Notes
 * on screen), then asks for {@code ecmvisual phone app} (a Counter app whose slot 1
 * is "Bump" is opened) and walks forward with hotkey slot 1 bound to W: the
 * server reports that slot 1 reached the open app while the client reports how
 * far it walked.
 */
public final class PhoneServerChecks {
    public static final BlockPos ORIGIN = new BlockPos(0, 170, 0);

    private static boolean built, typed, failed, offhandDone, hotkeyDone, openDone, appOpen;
    private static int appCount = -1;
    private static int finishTicks = -1;
    private static UUID phone;
    private static BlockPos dockPos;
    private static boolean dockReported, takenReported;
    private static UUID dockedPhone;

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
                .then(Commands.literal("thirdperson").executes(c -> {
                    ServerPlayer p = c.getSource().getPlayerOrException();
                    PhoneHost h = host(c.getSource().getServer());
                    if (h == null || h.state() != PhoneState.AWAKE || !p.getUUID().equals(h.holderId())
                            || !phone.equals(PhoneItemData.id(p.getMainHandItem()))) {
                        fail(c.getSource().getServer(), "third person: phone not awake in the player's main hand: " + h);
                        return 0;
                    }
                    EvansComputerMod.LOGGER.info("ECM_VISUAL_SERVER_PASS phone_third_person state={} visibility={}", h.state(), h.visibility());
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
                .then(Commands.literal("app").executes(c -> {
                    // A Counter app (slot 1 = "Bump", +100) opened on the phone: the open app the hotkey must reach.
                    PhoneHost h = host(c.getSource().getServer());
                    try {
                        installCounter(h);
                    } catch (Exception e) {
                        fail(c.getSource().getServer(), "can't install the counter app: " + e);
                        return 0;
                    }
                    h.instance().getUiManager().control("launch " + COUNTER);
                    return 1;
                }))
                .then(Commands.literal("finish").executes(c -> {
                    EvansComputerMod.LOGGER.info("ECM_VISUAL_SERVER_PASS phone_final");
                    finishTicks = 60;
                    return 1;
                }))));
    }

    static final String COUNTER = "dev.ecm.test.counter";

    private static void installCounter(PhoneHost h) throws java.io.IOException {
        java.nio.file.Path disk = h.instance().storagePath();
        java.nio.file.Path dir = disk.resolve("apps").resolve(COUNTER);
        java.nio.file.Files.createDirectories(dir);
        java.nio.file.Files.copy(com.example.evanscomputermod.wasm.WasmManager.getWasmBinPath().resolve("phonetest.wasm"),
                dir.resolve("main.wasm"), java.nio.file.StandardCopyOption.REPLACE_EXISTING);
        java.nio.file.Files.writeString(dir.resolve("app.toml"), "id = \"" + COUNTER + "\"\nname = \"Counter\"\nversion = \"1.0.0\"\n"
                + "entry = \"main.wasm\"\nhotkeys = [{ slot = 1, action = \"bump\", label = \"Bump\" }]\n");
        h.instance().getAppManager().reindex();
    }

    private static int countOf(PhoneHost h) {
        try {
            java.nio.file.Path f = h.instance().storagePath().resolve("var/apps").resolve(COUNTER).resolve("count");
            return java.nio.file.Files.exists(f) ? Integer.parseInt(java.nio.file.Files.readString(f).trim()) : -1;
        } catch (Exception e) {
            return -1;
        }
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
                // P5b: the phone boots into its shell (always GUI).
                var ui = h.instance() == null ? null : h.instance().getUiManager();
                var comp = ui == null ? null : ui.compositor();
                if (comp == null || ui.phone() == null || !comp.shellConnected() || !comp.layerTexts("home").contains("Notes")) return;
                typed = true;
                EvansComputerMod.LOGGER.info("ECM_VISUAL_SERVER_PASS phone_fixture phone={} display={}x{} home={}", phone,
                        h.display().getGfxWidth(), h.display().getGfxHeight(), comp.layerTexts("home"));
                return;
            }
            var comp = h.instance() == null ? null : h.instance().getUiManager().compositor();
            if (!openDone && comp != null && comp.windows().stream().anyMatch(w -> w.appId().equals("dev.ecm.notes") && w.focused() && w.mapped())) {
                openDone = true;
                EvansComputerMod.LOGGER.info("ECM_VISUAL_SERVER_PASS phone_open notes on screen, windows={}", comp.windows());
            }
            if (!appOpen && comp != null && countOf(h) >= 0
                    && comp.windows().stream().anyMatch(w -> w.appId().equals(COUNTER) && w.focused() && w.mapped())) {
                appOpen = true;
                appCount = countOf(h);
                EvansComputerMod.LOGGER.info("ECM_VISUAL_SERVER_PASS phone_app counter open at {}", appCount);
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
                dockedPhone = PhoneItemData.id(be.phoneStack());
            }
            if (dockReported && !takenReported && server.overworld().getBlockEntity(dockPos) instanceof ChargingDockBlockEntity be
                    && be.phoneStack().isEmpty() && !server.overworld().getBlockState(dockPos).getValue(ChargingDockBlock.ACTIVE)) {
                int slot = -1;
                for (int i = 0; i < 36; i++) if (dockedPhone != null && dockedPhone.equals(PhoneItemData.id(player.getInventory().getItem(i)))) slot = i;
                if (slot < 0) {
                    fail(server, "the dock is empty but the taken phone isn't in the player's inventory");
                    return;
                }
                takenReported = true;
                // What this player's client was sent: the dock's update tag now (empty dock, never an empty tag).
                var tag = be.getUpdateTag(server.registryAccess());
                if (tag.isEmpty() || tag.contains("Phone")) {
                    fail(server, "the update tag after taking the phone is " + tag);
                    return;
                }
                EvansComputerMod.LOGGER.info("ECM_VISUAL_SERVER_PASS phone_dock_taken dock empty, lamp off, phone in slot {}, update tag {}", slot, tag);
            }
            if (!hotkeyDone && appOpen) {
                // Slot 1 (bound to W on the client) reached the open app: "Bump" added 100.
                var ev = h.instance().getUiManager().phone().events();
                int now = countOf(h);
                if (ev.contains("hotkey 1 down -> " + COUNTER) && ev.contains("hotkey 1 up -> " + COUNTER) && now >= appCount + 100) {
                    hotkeyDone = true;
                    EvansComputerMod.LOGGER.info("ECM_VISUAL_SERVER_PASS phone_hotkey count {} -> {} events={}", appCount, now,
                            ev.stream().filter(x -> x.startsWith("hotkey")).toList());
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
