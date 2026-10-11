package com.example.evanscomputermod.testing;

//? if <=1.21.1 {
import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.block.ModBlocks;
import com.example.evanscomputermod.block.ScreenBlock;
import com.example.evanscomputermod.block.ScreenBlockEntity;
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
import net.minecraft.world.level.GameRules;
import net.minecraft.world.level.GameType;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.phys.Vec3;
import net.neoforged.neoforge.event.tick.ServerTickEvent;

/**
 * Screen render suite, server half ({@code -Decm.clientChecks.suite=screen}): a 4x4 Screen
 * cluster facing south with its terminal behind the bottom-left block, running
 * {@code gfxtest screen}. The hidden client ({@code ScreenClientChecks}) checks that every
 * block of the cluster shows the picture.
 */
public final class ScreenServerChecks {
  public static final int SIZE = 4;
  /** Bottom-left screen seen from the front; the cluster spans +x and +y from here. */
  public static final BlockPos ORIGIN = new BlockPos(-2, 170, 0);
  /** Distance of the camera from the screen face. */
  public static final double DISTANCE = 6;
  public static final BlockPos TERMINAL = ORIGIN.north();

  private static boolean built, booted, started, failed;
  private static int finishTicks = -1, waited;

  private ScreenServerChecks() {}

  /** The cluster's top-left block (seen from the front). */
  public static BlockPos anchor() {
    return ORIGIN.above(SIZE - 1);
  }

  /** Eye position of the camera: centred on the cluster, looking north at it. */
  public static Vec3 eye() {
    return new Vec3(
        ORIGIN.getX() + SIZE / 2.0, ORIGIN.getY() + SIZE / 2.0, ORIGIN.getZ() + 1 + DISTANCE);
  }

  public static void register(CommandDispatcher<CommandSourceStack> dispatcher) {
    if (!RadioVisualLayout.active("screen")) return;
    dispatcher.register(
        Commands.literal("ecmvisual")
            .then(
                Commands.literal("screen")
                    .then(
                        Commands.literal("finish")
                            .executes(
                                c -> {
                                  var level = c.getSource().getLevel();
                                  int active = 0;
                                  for (int col = 0; col < SIZE; col++)
                                    for (int row = 0; row < SIZE; row++)
                                      if (level.getBlockEntity(ORIGIN.offset(col, row, 0)) instanceof ScreenBlockEntity s
                                          && s.isActive()) active++;
                                  if (active != SIZE * SIZE) {
                                    failed = true;
                                    EvansComputerMod.LOGGER.error(
                                        "ECM_VISUAL_SERVER_FAIL only {} of {} screens active", active, SIZE * SIZE);
                                    c.getSource().getServer().halt(false);
                                    return 0;
                                  }
                                  EvansComputerMod.LOGGER.info("ECM_VISUAL_SERVER_PASS screen_cluster active={}", active);
                                  EvansComputerMod.LOGGER.info("ECM_VISUAL_SERVER_PASS screen_final");
                                  finishTicks = 60;
                                  return 1;
                                }))));
  }

  public static void tick(ServerTickEvent.Post event) {
    if (!RadioVisualLayout.active("screen") || failed) return;
    MinecraftServer server = event.getServer();
    if (finishTicks >= 0) {
      if (--finishTicks == 0) server.halt(false);
      return;
    }
    if (started || server.getPlayerList().getPlayers().isEmpty()) return;
    try {
      ServerLevel level = server.overworld();
      if (!built) {
        build(server, level);
        built = true;
        return;
      }
      if (!(level.getBlockEntity(TERMINAL) instanceof TerminalBlockEntity terminal))
        throw new IllegalStateException("terminal missing at " + TERMINAL);
      if (++waited > 20 * 90) throw new IllegalStateException("screen fixture timed out");
      if (!booted) {
        terminal.initializeWasm(); // what opening the GUI does
        booted = true;
        return;
      }
      if (!terminal.hasScreenCluster()
          || !ScenarioRun.screen(terminal.getDisplay()).contains("Welcome to Terminal OS")) return;
      terminal.onStringInput("gfxtest screen\n");
      started = true;
      EvansComputerMod.LOGGER.info(
          "ECM_VISUAL_SERVER_PASS screen_fixture anchorIsAnchor={}",
          level.getBlockEntity(anchor()) instanceof ScreenBlockEntity s && s.isAnchor());
    } catch (Throwable t) {
      failed = true;
      EvansComputerMod.LOGGER.error("ECM_VISUAL_SERVER_FAIL", t);
      server.halt(false);
    }
  }

  private static void build(MinecraftServer server, ServerLevel level) {
    var player = server.getPlayerList().getPlayers().get(0);
    server.getPlayerList().op(player.getGameProfile());
    level.getGameRules().getRule(GameRules.RULE_DAYLIGHT).set(false, server);
    level.getGameRules().getRule(GameRules.RULE_WEATHER_CYCLE).set(false, server);
    level.getGameRules().getRule(GameRules.RULE_DOMOBSPAWNING).set(false, server);
    level.setDayTime(6000);
    level.setWeatherParameters(12000, 0, false, false);
    int y = ORIGIN.getY();
    for (int x = ORIGIN.getX() - 4; x <= ORIGIN.getX() + SIZE + 4; x++)
      for (int z = ORIGIN.getZ() - 3; z <= ORIGIN.getZ() + DISTANCE + 4; z++) {
        level.setBlock(new BlockPos(x, y - 1, z), Blocks.SMOOTH_STONE.defaultBlockState(), 2);
        for (int yy = y; yy <= y + SIZE + 4; yy++)
          level.setBlock(new BlockPos(x, yy, z), Blocks.AIR.defaultBlockState(), 2);
      }
    level.setBlock(
        TERMINAL,
        ModBlocks.TERMINAL_BLOCK.get().defaultBlockState().setValue(TerminalBlock.FACING, Direction.NORTH),
        3);
    for (int c = 0; c < SIZE; c++)
      for (int r = 0; r < SIZE; r++)
        level.setBlock(
            ORIGIN.offset(c, r, 0),
            ModBlocks.SCREEN_BLOCK.get().defaultBlockState().setValue(ScreenBlock.FACING, Direction.SOUTH),
            3);
    player.setGameMode(GameType.CREATIVE);
    player.getAbilities().flying = true;
    player.onUpdateAbilities();
    Vec3 eye = eye();
    player.teleportTo(level, eye.x, eye.y - player.getEyeHeight(), eye.z, 180f, 0f);
  }
}
//?}
