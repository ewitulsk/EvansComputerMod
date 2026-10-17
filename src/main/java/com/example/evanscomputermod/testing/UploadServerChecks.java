package com.example.evanscomputermod.testing;

//? if <=1.21.1 {
import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.block.ModBlocks;
import com.example.evanscomputermod.block.TerminalBlock;
import com.example.evanscomputermod.block.TerminalBlockEntity;
import com.example.evanscomputermod.testing.scenario.ScenarioRun;
import com.mojang.brigadier.CommandDispatcher;
import net.minecraft.commands.CommandSourceStack;
import net.minecraft.commands.Commands;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.server.MinecraftServer;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.world.level.GameType;
import net.minecraft.world.level.block.Blocks;
import net.neoforged.neoforge.event.tick.ServerTickEvent;

import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;

/**
 * File upload suite, server half ({@code -Decm.clientChecks.suite=upload}, with the
 * {@code evanscomputermod-upload} sidecar in both runs' mods: {@code scripts/Test.ps1
 * -ClientChecks -ClientSuite upload -Upload}). A real multiplayer upload: a Terminal is
 * built in front of the joined player and its GUI opened; the hidden client
 * ({@code UploadClientChecks}) types {@code upload} on it, its sidecar "picks" a file (the
 * system property that stands in for the system dialog) and sends it over the network. This
 * half checks that the file arrived with the same bytes ({@code upload_terminal}), then that a
 * cancelled pick leaves nothing ({@code upload_cancelled}).
 */
public final class UploadServerChecks {
  public static final BlockPos TERMINAL = new BlockPos(0, 170, 0);
  /** What the client's file holds. */
  public static final String CONTENT = "Greetings from a real computer!";

  private static boolean built, booted, opened, failed, savedReported, cancelReported;
  private static int finishTicks = -1, waited;

  private UploadServerChecks() {}

  public static void register(CommandDispatcher<CommandSourceStack> dispatcher) {
    if (!RadioVisualLayout.active("upload")) return;
    dispatcher.register(Commands.literal("ecmvisual").then(Commands.literal("upload").then(Commands.literal("finish").executes(c -> {
      if (!savedReported || !cancelReported) {
        fail(c.getSource().getServer(), "finish before both upload checks passed");
        return 0;
      }
      EvansComputerMod.LOGGER.info("ECM_VISUAL_SERVER_PASS upload_final");
      finishTicks = 60;
      return 1;
    }))));
  }

  private static void fail(MinecraftServer server, String why) {
    failed = true;
    EvansComputerMod.LOGGER.error("ECM_VISUAL_SERVER_FAIL {}", why);
    server.halt(false);
  }

  public static void tick(ServerTickEvent.Post event) {
    if (!RadioVisualLayout.active("upload") || failed) return;
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
      if (!(level.getBlockEntity(TERMINAL) instanceof TerminalBlockEntity terminal))
        throw new IllegalStateException("terminal missing at " + TERMINAL);
      if (++waited > 20 * 150) throw new IllegalStateException("upload suite timed out");
      if (!booted) {
        terminal.initializeWasm();
        booted = true;
        return;
      }
      String screen = ScenarioRun.screen(terminal.getDisplay());
      if (!opened) {
        if (!screen.contains("Welcome to Terminal OS")) return;
        terminal.onStringInput("mkdir -p /home\n");
        server.getPlayerList().getPlayers().get(0).openMenu(terminal, TERMINAL);
        opened = true;
        EvansComputerMod.LOGGER.info("ECM_VISUAL_SERVER_PASS upload_fixture");
        return;
      }
      Path home = terminal.getComputer() == null ? null : terminal.getComputer().storagePath().resolve("home");
      if (!savedReported && screen.contains("upload: saved /home/postcard.txt") && home != null) {
        String got = Files.readString(home.resolve("postcard.txt"), StandardCharsets.UTF_8);
        if (!got.equals(CONTENT)) throw new IllegalStateException("postcard.txt holds '" + got + "'");
        savedReported = true;
        EvansComputerMod.LOGGER.info("ECM_VISUAL_SERVER_PASS upload_terminal bytes={}", got.length());
      }
      if (savedReported && !cancelReported && screen.contains("upload: cancelled") && home != null) {
        try (var s = Files.list(home)) {
          long n = s.count();
          if (n != 1) throw new IllegalStateException("after the cancelled upload /home holds " + n + " files");
        }
        cancelReported = true;
        EvansComputerMod.LOGGER.info("ECM_VISUAL_SERVER_PASS upload_cancelled");
      }
    } catch (Throwable t) {
      EvansComputerMod.LOGGER.error("ECM_VISUAL_SERVER_FAIL", t);
      failed = true;
      server.halt(false);
    }
  }

  private static void build(MinecraftServer server, ServerLevel level) {
    var player = server.getPlayerList().getPlayers().get(0);
    int y = TERMINAL.getY();
    for (int x = -4; x <= 4; x++)
      for (int z = -4; z <= 4; z++) {
        level.setBlock(new BlockPos(x, y - 1, z), Blocks.SMOOTH_STONE.defaultBlockState(), 2);
        for (int yy = y; yy <= y + 4; yy++) level.setBlock(new BlockPos(x, yy, z), Blocks.AIR.defaultBlockState(), 2);
      }
    level.setBlock(TERMINAL, ModBlocks.TERMINAL_BLOCK.get().defaultBlockState().setValue(TerminalBlock.FACING, Direction.SOUTH), 3);
    player.setGameMode(GameType.CREATIVE);
    player.teleportTo(level, 0.5, y, 2.5, 180f, 0f);
  }
}
//?}
