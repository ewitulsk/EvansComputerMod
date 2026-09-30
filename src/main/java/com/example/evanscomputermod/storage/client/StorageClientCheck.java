package com.example.evanscomputermod.storage.client;

//? if <=1.21.1 {

import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.block.ModBlocks;
import com.example.evanscomputermod.block.ScreenBlock;
import com.example.evanscomputermod.block.TerminalBlock;
import com.example.evanscomputermod.block.TerminalBlockEntity;
import com.example.evanscomputermod.block.TerminalScreen;
import com.example.evanscomputermod.computer.overlay.ItemOverlays;
import com.example.evanscomputermod.computer.overlay.ScreenTouch;
import com.example.evanscomputermod.computer.overlay.client.ClientItemOverlays;
import com.example.evanscomputermod.computer.overlay.client.ItemOverlayRenderer;
import com.example.evanscomputermod.storage.StorageContent;
import com.example.evanscomputermod.storage.core.CellContents;
import com.example.evanscomputermod.storage.core.CellTier;
import com.example.evanscomputermod.storage.device.DriveBlockEntity;
import com.example.evanscomputermod.storage.item.StorageCellItem;
import com.example.evanscomputermod.storage.ledger.StorageLedger;
import com.example.evanscomputermod.testing.scenario.ScenarioRun;
import com.mojang.blaze3d.platform.NativeImage;
import net.minecraft.client.Minecraft;
import net.minecraft.client.Screenshot;
import net.minecraft.client.gui.screens.AccessibilityOnboardingScreen;
import net.minecraft.client.gui.screens.TitleScreen;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.core.component.DataComponents;
import net.minecraft.server.MinecraftServer;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.server.level.ServerPlayer;
import net.minecraft.util.RandomSource;
import net.minecraft.world.Difficulty;
import net.minecraft.world.InteractionHand;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.item.Items;
import net.minecraft.world.item.enchantment.Enchantments;
import net.minecraft.world.item.component.ItemContainerContents;
import net.minecraft.world.level.GameRules;
import net.minecraft.world.level.GameType;
import net.minecraft.world.level.LevelSettings;
import net.minecraft.world.level.WorldDataConfiguration;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.HorizontalDirectionalBlock;
import net.minecraft.world.level.levelgen.WorldOptions;
import net.minecraft.world.level.levelgen.presets.WorldPresets;
import net.minecraft.world.phys.BlockHitResult;
import net.minecraft.world.phys.Vec3;
import net.neoforged.neoforge.client.event.ClientTickEvent;
import net.neoforged.neoforge.client.event.ScreenEvent;
import org.lwjgl.glfw.GLFW;

import java.nio.file.Files;
import java.nio.file.Path;
import java.util.List;
import java.util.UUID;

/**
 * A real-client check of the storage visuals, for development. It does
 * nothing unless the file {@code ecm-storage-client-check} is in the game
 * directory. Then it creates a world, builds a computer with a Screen, a Drive
 * full of cells and a Decoder, runs the {@code storage} app on the Screen and
 * in the terminal GUI, and saves screenshots (item overlays in the world, the
 * request dialog after a Screen touch, the GUI with a vanilla tooltip) to the
 * directory named in the file. It checks the new blocks and items bake without
 * missing textures, logs {@code ECM_STORAGE_CLIENT_PASS} or
 * {@code ECM_STORAGE_CLIENT_FAIL}, and quits.
 */
public final class StorageClientCheck {

    private static final Path FLAG = Path.of("ecm-storage-client-check");
    private static final long LIMIT_MS = 150_000;

    private static Path out;
    private static boolean worldRequested;
    private static int stage;
    private static int settle;
    private static long started;
    private static long lastGen;

    /** Server-side scene state, written on the server thread. */
    private static volatile BlockPos terminal;
    private static volatile int serverStage;

    private StorageClientCheck() {
    }

    static boolean enabled() {
        if (out != null) return true;
        if (!Files.exists(FLAG)) return false;
        try {
            String dir = Files.readString(FLAG).trim();
            out = Path.of(dir.isEmpty() ? "storage-client-check" : dir);
        } catch (Exception e) {
            out = Path.of("storage-client-check");
        }
        return true;
    }

    static void onScreen(ScreenEvent.Opening e) {
        // A fresh client opens on the accessibility onboarding screen instead of the title screen.
        if (worldRequested || !enabled()
                || !(e.getNewScreen() instanceof TitleScreen || e.getNewScreen() instanceof AccessibilityOnboardingScreen)) return;
        var title = e.getNewScreen();
        worldRequested = true;
        Minecraft mc = Minecraft.getInstance();
        mc.tell(() -> {
            String name = "ecm_storage_check_" + System.currentTimeMillis();
            GameRules rules = new GameRules();
            rules.getRule(GameRules.RULE_DAYLIGHT).set(false, null);
            rules.getRule(GameRules.RULE_DOMOBSPAWNING).set(false, null);
            LevelSettings settings = new LevelSettings(name, GameType.CREATIVE, false, Difficulty.PEACEFUL, true, rules,
                    WorldDataConfiguration.DEFAULT);
            mc.createWorldOpenFlows().createFreshLevel(name, settings, new WorldOptions(20260930L, false, false),
                    WorldPresets::createNormalWorldDimensions, title);
        });
    }

    static void tick(ClientTickEvent.Post e) {
        if (!enabled() || !worldRequested) return;
        Minecraft mc = Minecraft.getInstance();
        if (started == 0) started = System.currentTimeMillis();
        try {
            if (System.currentTimeMillis() - started > LIMIT_MS) {
                throw new IllegalStateException("timed out at stage " + stage + " (server stage " + serverStage + ")");
            }
            if (mc.level == null || mc.player == null || mc.getSingleplayerServer() == null) return;
            MinecraftServer server = mc.getSingleplayerServer();
            mc.options.pauseOnLostFocus = false;
            switch (stage) {
                case 0 -> {
                    UUID player = mc.player.getUUID();
                    server.execute(() -> build(server, player));
                    stage = 1;
                }
                case 1 -> {
                    server.execute(() -> serverTick(server));
                    if (serverStage >= 4 && ClientItemOverlays.get(terminal, ItemOverlays.TARGET_SCREEN) != null && ++settle > 60) {
                        settle = 0;
                        verifyModels(mc);
                        verifyScreenSync(mc, server);
                        capture(mc, "storage_screen");
                        server.execute(() -> touchFirstItem(server));
                        stage = 2;
                    }
                }
                case 2 -> {
                    ClientItemOverlays.Overlay o = ClientItemOverlays.get(terminal, ItemOverlays.TARGET_SCREEN);
                    boolean dimmed = o != null && o.entries.stream().allMatch(en -> (en.flags() & ItemOverlays.FLAG_DIMMED) != 0);
                    if (dimmed && ++settle > 30) {
                        settle = 0;
                        capture(mc, "storage_screen_dialog");
                        BlockPos t = terminal;
                        mc.gameMode.useItemOn(mc.player, InteractionHand.MAIN_HAND,
                                new BlockHitResult(Vec3.atCenterOf(t).add(0, 0, 0.5), Direction.SOUTH, t, false));
                        stage = 3;
                    }
                }
                case 3 -> {
                    if (mc.screen instanceof TerminalScreen) {
                        server.execute(() -> {
                            if (server.overworld().getBlockEntity(terminal) instanceof TerminalBlockEntity te) {
                                te.onStringInput("storage\n");
                            }
                        });
                        stage = 4;
                    }
                }
                case 4 -> {
                    if (ClientItemOverlays.get(terminal, ItemOverlays.TARGET_TERMINAL) != null && ++settle > 40) {
                        settle = 0;
                        float[] item = ItemOverlayRenderer.lastGuiItem;
                        if (item != null) {
                            double s = mc.getWindow().getGuiScale();
                            GLFW.glfwSetCursorPos(mc.getWindow().getWindow(), (item[0] + item[2] / 2) * s, (item[1] + item[2] / 2) * s);
                        }
                        stage = 5;
                    }
                }
                case 5 -> {
                    if (++settle > 15) {
                        capture(mc, "storage_terminal_gui");
                        EvansComputerMod.LOGGER.info("ECM_STORAGE_CLIENT_PASS screenshots in {}", out.toAbsolutePath());
                        stage = 6;
                        mc.stop();
                    }
                }
                default -> {
                }
            }
        } catch (Throwable t) {
            EvansComputerMod.LOGGER.error("ECM_STORAGE_CLIENT_FAIL", t);
            stage = 99;
            mc.stop();
        }
    }

    // ------------------------------------------------------------ server side (server thread)

    private static void build(MinecraftServer server, UUID playerId) {
        ServerLevel level = server.overworld();
        ServerPlayer player = server.getPlayerList().getPlayer(playerId);
        if (player == null) return;
        BlockPos b = new BlockPos(player.getBlockX(), 200, player.getBlockZ());
        for (int x = -5; x <= 5; x++) {
            for (int z = -4; z <= 8; z++) {
                level.setBlock(b.offset(x, -1, z), Blocks.SMOOTH_STONE.defaultBlockState(), Block.UPDATE_ALL);
                for (int y = 0; y < 5; y++) level.setBlock(b.offset(x, y, z), Blocks.AIR.defaultBlockState(), Block.UPDATE_ALL);
            }
        }
        level.setBlock(b, ModBlocks.TERMINAL_BLOCK.get().defaultBlockState().setValue(TerminalBlock.FACING, Direction.SOUTH), Block.UPDATE_ALL);
        for (int x = -3; x <= -1; x++) {
            for (int y = 0; y <= 1; y++) {
                level.setBlock(b.offset(x, y, 0), ModBlocks.SCREEN_BLOCK.get().defaultBlockState()
                        .setValue(ScreenBlock.FACING, Direction.SOUTH), Block.UPDATE_ALL);
            }
        }
        device(level, b.east(), StorageContent.DRIVE.get(), Direction.SOUTH);
        device(level, b.east(2), StorageContent.ENCODER.get(), Direction.SOUTH);
        device(level, b.north(), StorageContent.DECODER.get(), Direction.NORTH);
        level.setBlock(b.north(2), Blocks.CHEST.defaultBlockState(), Block.UPDATE_ALL);
        player.teleportTo(level, b.getX() - 0.5, b.getY(), b.getZ() + 5.5, 180f, 12f);
        terminal = b;
        serverStage = 1;
    }

    private static void device(ServerLevel level, BlockPos pos, Block block, Direction facing) {
        level.setBlock(pos, block.defaultBlockState().setValue(HorizontalDirectionalBlock.FACING, facing), Block.UPDATE_ALL);
    }

    private static void serverTick(MinecraftServer server) {
        ServerLevel level = server.overworld();
        BlockPos b = terminal;
        if (b == null || !(level.getBlockEntity(b) instanceof TerminalBlockEntity te)) return;
        switch (serverStage) {
            case 1 -> {
                if (!(level.getBlockEntity(b.east()) instanceof DriveBlockEntity drive) || !drive.isLive()) return;
                CellTier[] tiers = CellTier.values();
                for (int i = 0; i < 6; i++) {
                    drive.cellHandler().setStackInSlot(i, new ItemStack(StorageContent.CELLS.get(tiers[i % tiers.length]).get()));
                }
                serverStage = 2;
            }
            case 2 -> {
                if (!(level.getBlockEntity(b.east()) instanceof DriveBlockEntity drive) || drive.cells().size() < 6) return;
                StorageLedger ledger = StorageLedger.get(server);
                UUID main = StorageCellItem.cellId(drive.cellHandler().getStackInSlot(3));
                ItemStack sword = new ItemStack(Items.DIAMOND_SWORD);
                sword.enchant(level.registryAccess().lookupOrThrow(net.minecraft.core.registries.Registries.ENCHANTMENT)
                        .getOrThrow(Enchantments.SHARPNESS), 5);
                ItemStack box = new ItemStack(Items.CYAN_SHULKER_BOX);
                box.set(DataComponents.CONTAINER, ItemContainerContents.fromItems(List.of(new ItemStack(Items.DIAMOND, 64))));
                List<ItemStack> stock = List.of(new ItemStack(Items.IRON_INGOT), new ItemStack(Items.GOLD_INGOT),
                        new ItemStack(Items.DIAMOND), new ItemStack(Items.REDSTONE), new ItemStack(Items.OAK_LOG),
                        new ItemStack(Items.GLASS), new ItemStack(Items.TORCH), new ItemStack(Items.COBBLESTONE),
                        new ItemStack(Items.BREAD), new ItemStack(Items.ENDER_PEARL), sword, box,
                        new ItemStack(Items.CHEST), new ItemStack(Items.HOPPER), new ItemStack(Items.LAPIS_LAZULI),
                        new ItemStack(Items.EMERALD), new ItemStack(Items.COPPER_INGOT), new ItemStack(Items.QUARTZ),
                        new ItemStack(ModBlocks.TERMINAL_BLOCK_ITEM.get()), new ItemStack(StorageContent.DRIVE_ITEM.get()));
                RandomSource r = RandomSource.create(7);
                try {
                    CellContents c = ledger.get(main);
                    for (ItemStack s : stock) {
                        int k = ledger.intern(s);
                        c.add(k, Math.min(c.maxInsert(k), 1 + r.nextInt(3000)));
                    }
                    ledger.changed(main);
                } catch (Exception ex) {
                    throw new IllegalStateException(ex);
                }
                te.initializeWasm();
                serverStage = 3;
            }
            case 3 -> {
                if (ScenarioRun.screen(te.getDisplay()).contains("Welcome to Terminal OS") && te.hasScreenCluster()) {
                    te.onStringInput("storage --screen &\n");
                    serverStage = 4;
                }
            }
            default -> {
            }
        }
    }

    private static void touchFirstItem(MinecraftServer server) {
        ServerLevel level = server.overworld();
        BlockPos b = terminal;
        if (!(level.getBlockEntity(b) instanceof TerminalBlockEntity te) || te.getScreenDisplay() == null) return;
        var entries = te.getItemOverlays().entries(ItemOverlays.TARGET_SCREEN);
        if (entries.isEmpty() || te.getScreenClusterInfo() == null) return;
        var entry = entries.get(0);
        BlockPos anchor = te.getScreenClusterInfo().anchor();
        int cols = te.getScreenClusterInfo().cols(), rows = te.getScreenClusterInfo().rows();
        var d = te.getScreenDisplay();
        double u = (entry.x() + entry.size() / 2.0) / d.getGfxWidth() * cols;
        double v = (entry.y() + entry.size() / 2.0) / d.getGfxHeight() * rows;
        // South-facing screen: its picture starts at the anchor's (x, y+1, z+1) corner and runs east.
        Vec3 at = new Vec3(anchor.getX() + u, anchor.getY() + 1 - v, anchor.getZ() + 1);
        BlockPos hitBlock = BlockPos.containing(at.x, at.y, anchor.getZ());
        ServerPlayer player = server.getPlayerList().getPlayers().get(0);
        ScreenTouch.use(level.getBlockState(hitBlock), level, hitBlock, player, new BlockHitResult(at, Direction.SOUTH, hitBlock, false));
    }

    // ------------------------------------------------------------ client checks

    /** The anchor Screen's client framebuffer must match the server's (what was drawn is what's shown). */
    private static void verifyScreenSync(Minecraft mc, MinecraftServer server) {
        BlockPos t = terminal;
        byte[] serverPixels = server.submit(() -> server.overworld().getBlockEntity(t) instanceof TerminalBlockEntity te
                && te.getScreenDisplay() != null ? te.getScreenDisplay().getPixelData().clone() : null).join();
        var info = server.submit(() -> server.overworld().getBlockEntity(t) instanceof TerminalBlockEntity te
                ? te.getScreenClusterInfo() : null).join();
        if (serverPixels == null || info == null) throw new IllegalStateException("no screen on the server");
        if (!(mc.level.getBlockEntity(info.anchor()) instanceof com.example.evanscomputermod.block.ScreenBlockEntity anchor)
                || anchor.clientDisplay == null) {
            throw new IllegalStateException("no client display on the anchor screen");
        }
        if (!java.util.Arrays.equals(serverPixels, anchor.clientDisplay.getPixelData())) {
            throw new IllegalStateException("client screen framebuffer differs from the server's");
        }
    }

    private static void verifyModels(Minecraft mc) {
        for (Block block : new Block[] {StorageContent.DRIVE.get(), StorageContent.ENCODER.get(), StorageContent.DECODER.get()}) {
            var state = block.defaultBlockState();
            var quads = mc.getBlockRenderer().getBlockModel(state).getQuads(state, null, RandomSource.create(0));
            if (quads.isEmpty() || quads.stream().anyMatch(q -> q.getSprite().contents().name().getPath().contains("missing"))) {
                throw new IllegalStateException("missing geometry or texture for " + block);
            }
        }
        for (var item : List.of(StorageContent.STORAGE_MODULE.get(), StorageContent.CELLS.get(CellTier.K1).get(),
                StorageContent.CELLS.get(CellTier.K64).get(), StorageContent.COMPONENTS_BY_TIER.get(CellTier.K16).get(),
                StorageContent.DRIVE_ITEM.get())) {
            var model = mc.getItemRenderer().getModel(new ItemStack(item), mc.level, mc.player, 0);
            if (model.getParticleIcon().contents().name().getPath().contains("missing")) {
                throw new IllegalStateException("missing item texture for " + item);
            }
        }
    }

    private static void capture(Minecraft mc, String name) throws Exception {
        Files.createDirectories(out);
        try (NativeImage image = Screenshot.takeScreenshot(mc.getMainRenderTarget())) {
            var colors = new java.util.HashSet<Integer>();
            for (int y = 0; y < image.getHeight(); y += 8)
                for (int x = 0; x < image.getWidth(); x += 8) colors.add(image.getPixelRGBA(x, y));
            if (colors.size() < 24) throw new IllegalStateException("blank frame for " + name + ": " + colors.size() + " colours");
            image.writeToFile(out.resolve(name + ".png"));
            EvansComputerMod.LOGGER.info("ECM_STORAGE_CLIENT_SHOT {} colours={}", name, colors.size());
        }
    }
}
//?}
