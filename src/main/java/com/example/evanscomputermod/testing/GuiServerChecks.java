package com.example.evanscomputermod.testing;

//? if <=1.21.1 {
import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.block.ModBlocks;
import com.example.evanscomputermod.block.TerminalBlock;
import com.example.evanscomputermod.block.TerminalBlockEntity;
import com.example.evanscomputermod.computer.TerminalDisplay;
import com.example.evanscomputermod.computer.ui.Compositor;
import com.example.evanscomputermod.computer.ui.UiManager;
import com.example.evanscomputermod.testing.scenario.ScenarioRun;
import com.mojang.brigadier.CommandDispatcher;
import net.minecraft.commands.CommandSourceStack;
import net.minecraft.commands.Commands;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.network.chat.Component;
import net.minecraft.server.MinecraftServer;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.world.level.GameRules;
import net.minecraft.world.level.GameType;
import net.minecraft.world.level.block.Blocks;
import net.neoforged.neoforge.event.tick.ServerTickEvent;

import java.io.InputStream;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.List;

/**
 * GUI render suite, server half ({@code -Decm.clientChecks.suite=gui}): a Terminal whose
 * {@code /etc/ui.toml} has the test shell open Notes and {@code gba}; {@code gui} is typed,
 * and once both windows are tiled and gba draws, the player stands at the Terminal with its
 * screen open. The hidden client ({@code GuiClientChecks}) screenshots the desktop, clicks
 * into Notes' text field and types; the server checks the click reached the compositor (the
 * shell focused Notes) and the typed text changed Notes' pixels on the server's display.
 * Then the same Terminal restarts its GUI with the P3 desktop ({@code dev.ecm.desktop},
 * opening Settings and the Terminal app): the client screenshots the desktop, clicks the
 * taskbar's Apps button through the screen, and the server checks the launcher popup
 * opened (the client screenshots it too).
 */
public final class GuiServerChecks {
    public static final BlockPos TERMINAL = new BlockPos(4, 170, 4);
    static final String UI_TOML = String.join("\n",
            "shell = \"test-shell\"",
            "shell_args = [\"--launch\", \"notes\", \"--launch\", \"gba:arm.gba\"]",
            "");

    static final String DESKTOP_TOML = String.join("\n",
            "shell = \"dev.ecm.desktop\"",
            "shell_args = [\"--launch\", \"settings\", \"--launch\", \"terminal\"]",
            "");
    private static boolean built, booted, typedGui, ready, typedReported, failed, desktopStarted, desktopReady, launcherReported;
    private static int waited, finishTicks = -1;
    private static byte[] notesBefore;
    private static Compositor.WindowInfo notes;

    private GuiServerChecks() {}

    public static void register(CommandDispatcher<CommandSourceStack> dispatcher) {
        if (!RadioVisualLayout.active("gui")) return;
        dispatcher.register(Commands.literal("ecmvisual").then(Commands.literal("gui").then(Commands.literal("finish").executes(c -> {
            EvansComputerMod.LOGGER.info("ECM_VISUAL_SERVER_PASS gui_final");
            finishTicks = 40;
            return 1;
        }))));
    }

    private static void fail(MinecraftServer server, String why) {
        failed = true;
        EvansComputerMod.LOGGER.error("ECM_VISUAL_SERVER_FAIL {}", why);
        server.halt(false);
    }

    public static void tick(ServerTickEvent.Post event) {
        if (!RadioVisualLayout.active("gui") || failed) return;
        MinecraftServer server = event.getServer();
        if (finishTicks >= 0) {
            if (--finishTicks == 0) server.halt(false);
            return;
        }
        if (server.getPlayerList().getPlayers().isEmpty()) return;
        try {
            ServerLevel level = server.overworld();
            if (!built) {
                build(server, level);
                built = true;
                return;
            }
            if (!(level.getBlockEntity(TERMINAL) instanceof TerminalBlockEntity terminal)) throw new IllegalStateException("terminal missing");
            if (++waited > 20 * 120) throw new IllegalStateException("GUI fixture timed out");
            if (!booted) {
                prepareDisk(terminal);
                terminal.initializeWasm();
                booted = true;
                return;
            }
            if (!typedGui) {
                if (!ScenarioRun.screen(terminal.getDisplay()).contains("Welcome to Terminal OS")) return;
                terminal.onStringInput("gui\n");
                typedGui = true;
                return;
            }
            UiManager ui = terminal.getComputer().getUiManager();
            Compositor comp = ui.compositor();
            if (comp == null) return;
            if (!ready) {
                List<Compositor.WindowInfo> w = comp.windows().stream().filter(x -> x.mapped() && x.placed()).toList();
                if (w.size() != 2 || w.get(0).x() + w.get(0).w() > w.get(1).x() && w.get(1).x() + w.get(1).w() > w.get(0).x()) return;
                notes = w.stream().filter(x -> x.appId().equals("notes")).findFirst().orElse(null);
                Compositor.WindowInfo gba = w.stream().filter(x -> x.appId().equals("gba")).findFirst().orElse(null);
                if (notes == null || gba == null || !varied(terminal.getDisplay(), gba)) return;
                TerminalDisplay d = terminal.getDisplay();
                if (d.getDisplayMode() != 1 || d.getGfxWidth() != 640 || d.getPixelFormat() != TerminalDisplay.PIXEL_FORMAT_RGBA8888) return;
                EvansComputerMod.LOGGER.info("ECM_VISUAL_SERVER_PASS gui_fixture {}", ui.status().strip().replace('\n', ';'));
                EvansComputerMod.LOGGER.info("ECM_VISUAL_SERVER_PASS gui_desktop notes={},{} {}x{} gba={},{} {}x{}",
                        notes.x(), notes.y(), notes.w(), notes.h(), gba.x(), gba.y(), gba.w(), gba.h());
                notesBefore = region(d, notes);
                var player = server.getPlayerList().getPlayers().get(0);
                BlockPos stand = TERMINAL.north(2);
                player.teleportTo(level, stand.getX() + 0.5, stand.getY(), stand.getZ() + 0.5, 180f, 0f);
                player.openMenu(terminal, TERMINAL);
                // Where Notes' text field is (framebuffer pixels): the client clicks there.
                player.sendSystemMessage(Component.literal("ECMGUI ready " + (notes.x() + 60) + " " + (notes.y() + 21)
                        + " " + (gba.x() + gba.w() / 2) + " " + (gba.y() + gba.h() / 2)));
                ready = true;
                return;
            }
            if (!typedReported) {
                Compositor.WindowInfo n = comp.windows().stream().filter(x -> x.id() == notes.id()).findFirst().orElse(null);
                if (n == null || !n.focused()) return;
                byte[] now = region(terminal.getDisplay(), n);
                if (java.util.Arrays.equals(now, notesBefore)) return;
                typedReported = true;
                EvansComputerMod.LOGGER.info("ECM_VISUAL_SERVER_PASS gui_typed notes focused by the click; its pixels changed after typing");
                server.getPlayerList().getPlayers().get(0).sendSystemMessage(Component.literal("ECMGUI typed"));
                return;
            }
            // P3: the same Terminal with the desktop shell.
            if (!desktopStarted) {
                // The P2 session ends (its gba too, which would otherwise come back as a window).
                for (Compositor.WindowInfo w : comp.windows()) if (w.legacy()) terminal.getComputer().getProcessManager().kill(w.pid());
                ui.stop();
                Path dir = com.example.evanscomputermod.computer.ComputerStorage.path(terminal);
                Files.writeString(dir.resolve("etc/ui.toml"), DESKTOP_TOML);
                ui.start();
                desktopStarted = true;
                return;
            }
            if (!desktopReady) {
                Compositor c = ui.compositor();
                if (c == null || !c.shellConnected() || !c.layerTexts("taskbar").contains("Apps")) return;
                List<Compositor.WindowInfo> apps = c.windows().stream().filter(x -> !x.shell() && x.mapped() && x.placed()).toList();
                if (apps.stream().noneMatch(x -> x.appId().equals("settings")) || apps.stream().noneMatch(x -> x.appId().equals("terminal"))) return;
                Compositor.WindowInfo term = apps.stream().filter(x -> x.appId().equals("terminal")).findFirst().get();
                if (c.textsOf(term.id()).stream().noneMatch(t -> t.contains("logged in as"))) return;
                int[] at = c.locateLayerText("taskbar", "Apps", false);
                if (at == null) return;
                desktopReady = true;
                EvansComputerMod.LOGGER.info("ECM_VISUAL_SERVER_PASS gui_p3_desktop {}", ui.status().strip().replace('\n', ';'));
                server.getPlayerList().getPlayers().get(0).sendSystemMessage(Component.literal("ECMGUI desktop " + at[0] + " " + at[1]));
                return;
            }
            if (!launcherReported) {
                Compositor c = ui.compositor();
                if (c == null || c.locatePopupText("Text Editor", false) == null) return;
                launcherReported = true;
                EvansComputerMod.LOGGER.info("ECM_VISUAL_SERVER_PASS gui_p3_launcher the click on Apps opened the launcher popup");
                server.getPlayerList().getPlayers().get(0).sendSystemMessage(Component.literal("ECMGUI launcher"));
            }
        } catch (Throwable t) {
            EvansComputerMod.LOGGER.error("GUI checks failed", t);
            fail(server, t.toString());
        }
    }

    private static void prepareDisk(TerminalBlockEntity terminal) throws Exception {
        Path dir = com.example.evanscomputermod.computer.ComputerStorage.path(terminal);
        Files.createDirectories(dir.resolve("etc"));
        Files.writeString(dir.resolve("etc/ui.toml"), UI_TOML);
        try (InputStream in = GuiServerChecks.class.getResourceAsStream("/evanscomputermod/scenarios/gui/arm.gba")) {
            if (in == null) throw new IllegalStateException("arm.gba missing from the jar");
            Files.write(dir.resolve("arm.gba"), in.readAllBytes());
        }
    }

    /** The RGBA bytes of a window's text-field strip on the server's display. */
    private static byte[] region(TerminalDisplay d, Compositor.WindowInfo w) {
        byte[] px;
        synchronized (d) {
            px = d.getPixelData();
            if (px == null || d.getGfxWidth() != 640) return new byte[0];
            px = px.clone();
        }
        byte[] out = new byte[w.w() * 30 * 4];
        for (int y = 0; y < 30; y++) {
            System.arraycopy(px, ((w.y() + 6 + y) * 640 + w.x()) * 4, out, y * w.w() * 4, w.w() * 4);
        }
        return out;
    }

    private static boolean varied(TerminalDisplay d, Compositor.WindowInfo w) {
        byte[] px = d.getPixelData();
        if (px == null || d.getGfxWidth() != 640) return false;
        int first = -1;
        for (int y = w.y() + 4; y < w.y() + w.h() - 4; y += 3) {
            for (int x = w.x() + 4; x < w.x() + w.w() - 4; x += 3) {
                int i = (y * 640 + x) * 4;
                int c = (px[i] & 0xFF) << 16 | (px[i + 1] & 0xFF) << 8 | (px[i + 2] & 0xFF);
                if (first < 0) first = c;
                else if (c != first) return true;
            }
        }
        return false;
    }

    private static void build(MinecraftServer server, ServerLevel level) {
        var player = server.getPlayerList().getPlayers().get(0);
        server.getPlayerList().op(player.getGameProfile());
        level.getGameRules().getRule(GameRules.RULE_DAYLIGHT).set(false, server);
        level.getGameRules().getRule(GameRules.RULE_WEATHER_CYCLE).set(false, server);
        level.getGameRules().getRule(GameRules.RULE_DOMOBSPAWNING).set(false, server);
        level.setDayTime(6000);
        for (int x = TERMINAL.getX() - 3; x <= TERMINAL.getX() + 3; x++)
            for (int z = TERMINAL.getZ() - 4; z <= TERMINAL.getZ() + 2; z++) {
                level.setBlock(new BlockPos(x, TERMINAL.getY() - 1, z), Blocks.SMOOTH_STONE.defaultBlockState(), 2);
                for (int y = TERMINAL.getY(); y <= TERMINAL.getY() + 3; y++) level.setBlock(new BlockPos(x, y, z), Blocks.AIR.defaultBlockState(), 2);
            }
        level.setBlock(TERMINAL, ModBlocks.TERMINAL_BLOCK.get().defaultBlockState().setValue(TerminalBlock.FACING, Direction.NORTH), 3);
        player.setGameMode(GameType.CREATIVE);
        BlockPos stand = TERMINAL.north(2);
        player.teleportTo(level, stand.getX() + 0.5, stand.getY(), stand.getZ() + 0.5, 180f, 0f);
    }
}
//?}
