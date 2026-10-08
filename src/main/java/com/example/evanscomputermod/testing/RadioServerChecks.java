package com.example.evanscomputermod.testing;

//? if <=1.21.1 {
import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.block.ModBlocks;
import com.example.evanscomputermod.radio.power.BurnerGeneratorBlockEntity;
import com.mojang.brigadier.CommandDispatcher;
import com.mojang.brigadier.arguments.StringArgumentType;
import java.util.ArrayList;
import java.util.stream.Collectors;
import net.minecraft.commands.*;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.server.MinecraftServer;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.world.entity.decoration.ItemFrame;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.item.Items;
import net.minecraft.world.level.GameRules;
import net.minecraft.world.level.GameType;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.phys.AABB;
import net.neoforged.neoforge.event.tick.ServerTickEvent;

/**
 * Radio render suite, server half (selected with {@code -Decm.clientChecks.suite=radio}):
 * builds a flat, daylit display of every registered radio block and item, waits for
 * block entities to reach their steady state, asserts each placed state, and serves
 * the camera views the hidden client ({@code RadioClientChecks}) asks for.
 */
public final class RadioServerChecks {
  /** Ticks block entities get to settle (burner lights, AP joins its cable) before asserting. */
  private static final int SETTLE_TICKS = 60;

  private static RadioVisualLayout.Layout layout;
  private static boolean built, ready, failed;
  private static int settleTicks = -1, finishTicks = -1;

  private RadioServerChecks() {}

  public static void register(CommandDispatcher<CommandSourceStack> dispatcher) {
    if (!RadioVisualLayout.active("radio")) return;
    dispatcher.register(
        Commands.literal("ecmvisual")
            .then(
                Commands.literal("radio")
                    .then(
                        Commands.literal("view")
                            .then(
                                Commands.argument("case", StringArgumentType.word())
                                    .executes(
                                        c ->
                                            !ready
                                                ? notReady(c.getSource())
                                                : guarded(
                                                c.getSource().getServer(),
                                                () -> view(c.getSource(), StringArgumentType.getString(c, "case"))))))
                    .then(
                        Commands.literal("finish")
                            .executes(c -> guarded(c.getSource().getServer(), () -> finish(c.getSource()))))));
  }

  /** The client may ask before block entities settled; it retries, so this is not a failure. */
  private static int notReady(CommandSourceStack source) {
    source.sendFailure(net.minecraft.network.chat.Component.literal("radio fixture is not ready yet"));
    return 0;
  }

  private interface Body {
    void run() throws Exception;
  }

  private static int guarded(MinecraftServer server, Body body) {
    try {
      body.run();
      return 1;
    } catch (Throwable t) {
      fail(server, t);
      return 0;
    }
  }

  private static void fail(MinecraftServer server, Throwable t) {
    failed = true;
    EvansComputerMod.LOGGER.error("ECM_VISUAL_SERVER_FAIL", t);
    server.halt(false);
  }

  private static void check(boolean condition, String message) {
    if (!condition) throw new IllegalStateException(message);
  }

  public static void tick(ServerTickEvent.Post event) {
    if (!RadioVisualLayout.active("radio") || failed) return;
    var server = event.getServer();
    if (finishTicks >= 0) {
      if (--finishTicks == 0) server.halt(false);
      return;
    }
    if (ready || server.getPlayerList().getPlayers().isEmpty()) return;
    try {
      var level = server.overworld();
      if (!built) {
        build(server, level);
        built = true;
        settleTicks = SETTLE_TICKS;
        return;
      }
      if (--settleTicks > 0) return;
      assertBlocks(level, layout.blocks());
      int frames = assertFrames(level);
      EvansComputerMod.LOGGER.info(
          "ECM_VISUAL_SERVER_PASS radio_fixture blocks={} items={} frames={}",
          layout.blocks().size(),
          layout.items().size(),
          frames);
      ready = true;
    } catch (Throwable t) {
      fail(server, t);
    }
  }

  private static void build(MinecraftServer server, ServerLevel level) {
    layout = RadioVisualLayout.compute();
    var player = server.getPlayerList().getPlayers().get(0);
    server.getPlayerList().op(player.getGameProfile());
    level.getGameRules().getRule(GameRules.RULE_DAYLIGHT).set(false, server);
    level.getGameRules().getRule(GameRules.RULE_WEATHER_CYCLE).set(false, server);
    level.getGameRules().getRule(GameRules.RULE_DOMOBSPAWNING).set(false, server);
    level.setDayTime(6000);
    level.setWeatherParameters(12000, 0, false, false);

    int y = RadioVisualLayout.Y, z = RadioVisualLayout.Z;
    int x0 = RadioVisualLayout.ITEM_X0 - 3, x1 = layout.rowEnd() + 4;
    int zFar =
        (int)
                Math.ceil(
                    layout.views().stream().mapToDouble(v -> v.feet().z).max().orElse(z + 6))
            + 3;
    for (int x = x0; x <= x1; x++)
      for (int zz = z - 3; zz <= zFar; zz++) {
        level.setBlock(new BlockPos(x, y - 1, zz), Blocks.SMOOTH_STONE.defaultBlockState(), 2);
        for (int yy = y; yy <= y + 10; yy++)
          level.setBlock(new BlockPos(x, yy, zz), Blocks.AIR.defaultBlockState(), 2);
      }

    var names = layout.blocks().stream().map(RadioVisualLayout.Placed::label).collect(Collectors.joining(" "));
    EvansComputerMod.LOGGER.info("ECM_VISUAL_RADIO_BLOCKS {}", names);
    EvansComputerMod.LOGGER.info(
        "ECM_VISUAL_RADIO_ITEMS {}",
        layout.items().stream().map(Object::toString).collect(Collectors.joining(" ")));
    EvansComputerMod.LOGGER.info(
        "ECM_VISUAL_RADIO_SKIPPED (not registered) {}", String.join(" ", layout.skipped()));
    EvansComputerMod.LOGGER.info(
        "ECM_VISUAL_RADIO_CASES {}",
        layout.views().stream().map(RadioVisualLayout.View::name).collect(Collectors.joining(",")));

    for (var placed : layout.blocks()) {
      if (placed.setup() == RadioVisualLayout.Setup.CABLE_BELOW)
        level.setBlock(placed.pos().below(), ModBlocks.NETWORK_CABLE.get().defaultBlockState(), 3);
      level.setBlock(placed.pos(), placed.state(), 3);
      if (placed.setup() == RadioVisualLayout.Setup.FUEL) {
        check(
            level.getBlockEntity(placed.pos()) instanceof BurnerGeneratorBlockEntity,
            "burner generator at " + placed.pos() + " has no block entity");
        var burner = (BurnerGeneratorBlockEntity) level.getBlockEntity(placed.pos());
        burner.fuel().setStackInSlot(0, new ItemStack(Items.COAL_BLOCK, 16));
      }
    }

    // Item wall: frames on smooth stone, facing the camera.
    for (int i = 0; i < layout.items().size(); i++) {
      var pos = layout.framePos(i);
      level.setBlock(pos.north(), Blocks.SMOOTH_STONE.defaultBlockState(), 3);
      var frame = new ItemFrame(level, pos, Direction.SOUTH);
      frame.setItem(new ItemStack(BuiltInRegistries.ITEM.get(layout.items().get(i))));
      frame.setInvulnerable(true);
      level.addFreshEntity(frame);
    }

    player.setGameMode(GameType.CREATIVE);
    player.getAbilities().flying = true;
    player.onUpdateAbilities();
    var first = layout.views().get(0);
    // Park near (not at) the first view: the client only captures after a view command moved it.
    player.teleportTo(
        level, first.feet().x, first.feet().y, first.feet().z + 2, first.yaw(), first.pitch());
  }

  private static void assertBlocks(ServerLevel level, java.util.List<RadioVisualLayout.Placed> blocks) {
    var wrong = new ArrayList<String>();
    for (var placed : blocks) {
      var actual = level.getBlockState(placed.pos());
      if (!placed.matches(actual)) wrong.add(placed.label() + " at " + placed.pos() + " is " + actual);
    }
    check(wrong.isEmpty(), "Placed radio states changed: " + String.join("; ", wrong));
  }

  private static int assertFrames(ServerLevel level) {
    int found = 0;
    for (int i = 0; i < layout.items().size(); i++) {
      var pos = layout.framePos(i);
      var item = BuiltInRegistries.ITEM.get(layout.items().get(i));
      var frames = level.getEntitiesOfClass(ItemFrame.class, new AABB(pos));
      check(
          frames.stream().anyMatch(f -> f.getItem().is(item)),
          "No item frame showing " + layout.items().get(i) + " at " + pos);
      found++;
    }
    return found;
  }

  private static void view(CommandSourceStack source, String name) throws Exception {
    check(ready, "radio fixture is not ready");
    var player = source.getPlayerOrException();
    var view = layout.view(name);
    if (name.equals(RadioVisualLayout.ITEMS)) assertFrames(source.getLevel());
    else assertBlocks(source.getLevel(), view.blocks());
    player.teleportTo(
        source.getLevel(), view.feet().x, view.feet().y, view.feet().z, view.yaw(), view.pitch());
    EvansComputerMod.LOGGER.info(
        "ECM_VISUAL_SERVER_PASS {} blocks={}",
        name,
        view.blocks().stream().map(RadioVisualLayout.Placed::label).collect(Collectors.joining(" ")));
  }

  private static void finish(CommandSourceStack source) {
    check(ready, "radio fixture is not ready");
    assertBlocks(source.getLevel(), layout.blocks());
    assertFrames(source.getLevel());
    EvansComputerMod.LOGGER.info("ECM_VISUAL_SERVER_PASS radio_final");
    finishTicks = 60;
  }
}
//?}
