package com.example.evanscomputermod.testing;

//? if <=1.21.1 {
import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.block.*;
import com.example.evanscomputermod.item.ModItems;
import com.example.evanscomputermod.worldgen.TechVillageLocator;
import com.mojang.brigadier.CommandDispatcher;
import net.minecraft.commands.*;
import net.minecraft.core.*;
import net.minecraft.server.level.*;
import net.minecraft.world.level.GameType;
import net.minecraft.world.level.block.Blocks;
import net.neoforged.neoforge.event.tick.ServerTickEvent;

/** Isolated normal-world fixtures and server assertions paired with client renders. */
public final class TechServerChecks {
  private static boolean built, failed;
  private static long start;
  private static TechVillageLocator.Visit village;
  private static int finishTicks=-1;

  public static void register(CommandDispatcher<CommandSourceStack> dispatcher) {
    if (!Boolean.getBoolean("ecm.clientChecks")) return;
    dispatcher.register(
        Commands.literal("ecmvisual")
            .then(
                Commands.literal("disconnect")
                    .executes(
                        c -> {
                          var l = c.getSource().getLevel();
                          l.removeBlock(new BlockPos(4, 108, 0), false);
                          check(
                              !l.getBlockState(new BlockPos(3, 108, 0))
                                  .getValue(NetworkCableBlock.EAST),
                              "east arm did not disconnect");
                          EvansComputerMod.LOGGER.info("ECM_VISUAL_SERVER_PASS fiber_disconnected");
                          return 1;
                        }))
            .then(
                Commands.literal("repair")
                    .executes(
                        c -> {
                          var l = c.getSource().getLevel();
                          l.setBlock(
                              new BlockPos(4, 108, 0),
                              ModBlocks.FIBER_SPAN.get().defaultBlockState(),
                              3);
                          check(
                              l.getBlockState(new BlockPos(3, 108, 0))
                                  .getValue(NetworkCableBlock.EAST),
                              "east arm did not repair");
                          EvansComputerMod.LOGGER.info("ECM_VISUAL_SERVER_PASS fiber_repaired");
                          return 1;
                        }))
            .then(
                Commands.literal("finish")
                    .executes(
                        c -> {
                          var player = c.getSource().getPlayerOrException();
                          check(
                              player
                                      .position()
                                      .distanceTo(
                                          net.minecraft.world.phys.Vec3.atCenterOf(
                                              village.arrival()))
                                  < 6,
                              "village command landed at the wrong position");
                          EvansComputerMod.LOGGER.info("ECM_VISUAL_SERVER_PASS village_arrival");
                          var commands = c.getSource().getServer().getCommands();
                          var source = player.createCommandSourceStack();
                          for (String command :
                              new String[] {
                                "ecm scenario commands router_home",
                                "ecm scenario spawn router_bgp_pair manual",
                                "ecm scenario links",
                                "ecm scenario link ring1 down",
                                "ecm scenario link ring1 up",
                                "ecm scenario clear"
                              })
                            check(
                                commands.getDispatcher().execute(command, source) > 0,
                                "Scenario command failed: " + command);
                          EvansComputerMod.LOGGER.info("ECM_VISUAL_SERVER_PASS scenario_commands");
                          // Give freshly cleared fixture chunks and their asynchronous
                          // IO time to settle before the server begins its unload loop.
                          finishTicks=60;
                          return 1;
                        })));
  }

  private static void check(boolean condition, String message) {
    if (!condition) throw new IllegalStateException(message);
  }

  public static void tick(ServerTickEvent.Post event) {
    if(finishTicks>=0){if(--finishTicks==0)event.getServer().halt(false);return;}
    if (!Boolean.getBoolean("ecm.clientChecks") || failed || built) return;
    var server = event.getServer();
    if (server.getPlayerList().getPlayers().isEmpty()) return;
    if (start == 0) start = System.currentTimeMillis();
    try {
      var level = server.overworld();
      var player = server.getPlayerList().getPlayers().get(0);
      server.getPlayerList().op(player.getGameProfile());
      village = TechVillageLocator.visit(level, 3);
      check(
          village.computers().size() == 15,
          "natural village has "
              + village.computers().size()
              + " distinct computers instead of 15");
      check(System.currentTimeMillis() - start < 55_000, "natural generation exceeded 55 seconds");
      EvansComputerMod.LOGGER.info(
          "ECM_VISUAL_SERVER_PASS natural_village seed={} arrival={} computers={}",
          level.getSeed(),
          village.arrival(),
          village.computers().size());
      for (int x = -5; x <= 16; x++)
        for (int z = -4; z <= 16; z++)
          level.setBlock(new BlockPos(x, 103, z), Blocks.SMOOTH_STONE.defaultBlockState(), 3);
      for (int y = 104; y < 108; y++)
        level.setBlock(new BlockPos(0, y, 0), ModBlocks.UTILITY_POLE.get().defaultBlockState(), 3);
      for (int x = 0; x <= 9; x++)
        level.setBlock(new BlockPos(x, 108, 0), ModBlocks.FIBER_SPAN.get().defaultBlockState(), 3);
      level.setBlock(new BlockPos(3, 108, 1), ModBlocks.FIBER_SPAN.get().defaultBlockState(), 3);
      level.setBlock(new BlockPos(3, 109, 0), ModBlocks.FIBER_SPAN.get().defaultBlockState(), 3);
      level.setBlock(
          new BlockPos(9, 107, 0),
          ModBlocks.FIBER_PATCH_PANEL
              .get()
              .defaultBlockState()
              .setValue(FiberPatchPanelBlock.FACING, Direction.SOUTH),
          3);
      var frame =
          new net.minecraft.world.entity.decoration.ItemFrame(
              level, new BlockPos(12, 106, 0), Direction.SOUTH);
      level.setBlock(new BlockPos(12, 106, -1), Blocks.SMOOTH_STONE.defaultBlockState(), 3);
      frame.setItem(ModItems.ALWAYS_ON_MODULE.get().getDefaultInstance());
      level.addFreshEntity(frame);
      var middle = level.getBlockState(new BlockPos(3, 108, 0));
      check(
          middle.getValue(NetworkCableBlock.EAST)
              && middle.getValue(NetworkCableBlock.WEST)
              && middle.getValue(NetworkCableBlock.SOUTH)
              && middle.getValue(NetworkCableBlock.UP),
          "neighbor state did not connect");
      check(
          !level.getBlockState(new BlockPos(9, 108, 0)).getValue(NetworkCableBlock.EAST),
          "isolated east end rendered an arm");
      check(
          !level.getBlockState(new BlockPos(0, 104, 0)).getValue(FiberInfrastructureBlock.TOP)
              && level
                  .getBlockState(new BlockPos(0, 107, 0))
                  .getValue(FiberInfrastructureBlock.TOP),
          "stacked pole cap state incorrect");
      player.setGameMode(GameType.CREATIVE);
      player.getAbilities().flying = true;
      player.onUpdateAbilities();
      player.teleportTo(level, 6, 104.5, 9.5, 180, -5);
      built = true;
      EvansComputerMod.LOGGER.info("ECM_VISUAL_SERVER_PASS fiber_connected");
    } catch (Throwable t) {
      failed = true;
      EvansComputerMod.LOGGER.error("ECM_VISUAL_SERVER_FAIL", t);
      server.halt(false);
    }
  }
}
//?}
