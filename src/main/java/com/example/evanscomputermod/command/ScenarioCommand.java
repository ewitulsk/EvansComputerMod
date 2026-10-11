package com.example.evanscomputermod.command;

import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.testing.scenario.Scenario;
import com.example.evanscomputermod.testing.scenario.ScenarioRun;
import com.example.evanscomputermod.testing.scenario.SwitchScenarios;
import com.mojang.brigadier.CommandDispatcher;
import com.mojang.brigadier.arguments.StringArgumentType;
import com.mojang.brigadier.context.CommandContext;
import com.mojang.brigadier.suggestion.SuggestionProvider;
import java.util.ArrayList;
import java.util.List;
import net.minecraft.commands.CommandSourceStack;
import net.minecraft.commands.Commands;
import net.minecraft.commands.SharedSuggestionProvider;
import net.minecraft.core.BlockPos;
import net.minecraft.network.chat.Component;
//? if >=26.1 {
import net.minecraft.server.permissions.PermissionCheck;
import net.minecraft.server.permissions.Permissions;
//?}
import net.neoforged.neoforge.event.server.ServerStoppingEvent;
import net.neoforged.neoforge.event.tick.ServerTickEvent;

/**
 * Debug scenarios in the world (op only).
 *
 * <pre>
 *   /ecm scenario list
 *   /ecm scenario spawn &lt;name&gt; [auto|manual|fast]
 *       build the layout 3 blocks south of you (screens face you), boot the
 *       terminals, and (auto, the default) type the script into them,
 *       reporting each step in chat. manual = build and boot only, then
 *       print the commands to type yourself. fast = auto without pacing.
 *   /ecm scenario commands &lt;name&gt;   print the script
 *   /ecm scenario rerun               rebuild the last scenario in place and run it again
 *   /ecm scenario status
 *   /ecm scenario clear               remove everything spawned
 * </pre>
 *
 * Spawning clears the layout's bounding box to air first. Scenarios come from {@code
 * SwitchScenarios} and, on 1.21.1, {@code SensorScenarios} ({@code lidar_room}).
 */
public final class ScenarioCommand {
  private static final int PACE_TICKS = 4;

  private record Active(ScenarioRun run, CommandSourceStack source, String mode) {}

  private static final List<Active> ACTIVE = new ArrayList<>();
  private static Active last;

  private static final SuggestionProvider<CommandSourceStack> NAMES =
      (ctx, b) -> SharedSuggestionProvider.suggest(catalog().keySet(), b);
  private static final SuggestionProvider<CommandSourceStack> MODES =
      (ctx, b) -> SharedSuggestionProvider.suggest(List.of("auto", "manual", "fast"), b);

  public static void register(CommandDispatcher<CommandSourceStack> dispatcher) {
    dispatcher.register(
        Commands.literal("ecm")
            //? if >=26.1 {
            .requires(
                Commands.hasPermission(
                    new PermissionCheck.Require(Permissions.COMMANDS_GAMEMASTER)))
            //?} else
            /*.requires(src -> src.hasPermission(2))*/
            .then(
                Commands.literal("scenario")
                    .then(Commands.literal("list").executes(ScenarioCommand::list))
                    .then(
                        Commands.literal("spawn")
                            .then(
                                Commands.argument("name", StringArgumentType.word())
                                    .suggests(NAMES)
                                    .executes(c -> spawn(c, "auto"))
                                    .then(
                                        Commands.argument("mode", StringArgumentType.word())
                                            .suggests(MODES)
                                            .executes(
                                                c ->
                                                    spawn(
                                                        c,
                                                        StringArgumentType.getString(c, "mode"))))))
                    .then(
                        Commands.literal("commands")
                            .then(
                                Commands.argument("name", StringArgumentType.word())
                                    .suggests(NAMES)
                                    .executes(ScenarioCommand::commands)))
                    .then(Commands.literal("rerun").executes(ScenarioCommand::rerun))
                    .then(Commands.literal("status").executes(ScenarioCommand::status))
                    .then(
                        Commands.literal("links")
                            .executes(
                                c -> {
                                  if (last == null) return 0;
                                  for (var line :
                                      com.example.evanscomputermod.testing.scenario.RouterScenarios
                                          .patchLeads(last.run())) say(c.getSource(), line);
                                  return 1;
                                }))
                    .then(
                        Commands.literal("link")
                            .then(
                                Commands.argument("lead", StringArgumentType.word())
                                    .then(
                                        Commands.argument("state", StringArgumentType.word())
                                            .suggests(
                                                (ctx, b) ->
                                                    SharedSuggestionProvider.suggest(
                                                        List.of("up", "down"), b))
                                            .executes(
                                                c -> {
                                                  if (last == null) return 0;
                                                  String state =
                                                      StringArgumentType.getString(c, "state");
                                                  if (!state.equals("up")
                                                      && !state.equals("down")) {
                                                    say(c.getSource(), "Use up or down.");
                                                    return 0;
                                                  }
                                                  String lead =
                                                      StringArgumentType.getString(c, "lead");
                                                  boolean ok =
                                                      com.example.evanscomputermod.testing.scenario
                                                          .RouterScenarios.patch(
                                                          last.run(), lead, state.equals("up"));
                                                  say(
                                                      c.getSource(),
                                                      ok
                                                          ? lead + " " + state
                                                          : "Unknown patch lead; use /ecm scenario"
                                                                + " links.");
                                                  return ok ? 1 : 0;
                                                }))))
                    .then(Commands.literal("clear").executes(ScenarioCommand::clear))));
  }

  /** Every spawnable scenario: switching, plus sensors on 1.21.1. */
  private static java.util.Map<String, Scenario> catalog() {
    java.util.Map<String, Scenario> all = new java.util.LinkedHashMap<>(SwitchScenarios.ALL);
    all.putAll(com.example.evanscomputermod.testing.scenario.RouterScenarios.ALL);
    //? if <=1.21.1 {
    all.putAll(com.example.evanscomputermod.testing.scenario.SensorScenarios.ALL);
    all.putAll(com.example.evanscomputermod.testing.scenario.RadioScenarios.ALL);
    //?}
    return all;
  }

  private static void say(CommandSourceStack src, String msg) {
    src.sendSystemMessage(Component.literal(msg));
  }

  private static int list(CommandContext<CommandSourceStack> c) {
    for (Scenario s : catalog().values()) {
      say(c.getSource(), "§f" + s.name + " §7- " + s.description);
    }
    return catalog().size();
  }

  private static Scenario lookup(CommandContext<CommandSourceStack> c) {
    String name = StringArgumentType.getString(c, "name");
    Scenario s = catalog().get(name);
    if (s == null) say(c.getSource(), "§cNo scenario '" + name + "'. Try /ecm scenario list");
    return s;
  }

  private static int spawn(CommandContext<CommandSourceStack> c, String mode) {
    Scenario s = lookup(c);
    if (s == null) return 0;
    if (!List.of("auto", "manual", "fast").contains(mode)) {
      say(c.getSource(), "§cmode must be auto, manual or fast");
      return 0;
    }
    CommandSourceStack src = c.getSource();
    BlockPos me = BlockPos.containing(src.getPosition());
    BlockPos min = s.min(), max = s.max();
    // Centred on the player in x, starting 3 blocks south, cables at foot level.
    BlockPos origin =
        new BlockPos(
            me.getX() - (min.getX() + max.getX()) / 2,
            me.getY() - min.getY(),
            me.getZ() + 3 - min.getZ());
    start(s, src, origin, mode);
    return 1;
  }

  private static void start(Scenario s, CommandSourceStack src, BlockPos origin, String mode) {
    ScenarioRun run =
        new ScenarioRun(
            s,
            src.getLevel(),
            origin,
            msg -> {
              say(src, msg);
              EvansComputerMod.LOGGER.info("[scenario {}] {}", s.name, msg.replaceAll("§.", ""));
            },
            mode.equals("auto") ? PACE_TICKS : 0,
            !mode.equals("manual"));
    run.build();
    Active a = new Active(run, src, mode);
    ACTIVE.add(a);
    last = a;
    say(src, "§e" + s.name + "§f: " + s.description);
    legend(src, s, run);
    if (mode.equals("manual")) {
      say(
          src,
          "§7Booting. Then type these on the terminals (or /ecm scenario commands "
              + s.name
              + "):");
      printWalkthrough(src, s);
    } else {
      say(src, "§7Booting, then running " + s.steps.size() + " steps. Look south.");
    }
  }

  /** Where each terminal is and which cable joins which ports. */
  private static void legend(CommandSourceStack src, Scenario s, ScenarioRun run) {
    for (Scenario.Node n : s.nodes.values()) {
      say(
          src,
          "§7  "
              + n.name()
              + " §f"
              + (n.role() == Scenario.Role.SWITCH
                  ? "switch"
                  : n.ip() == null || "-".equals(n.ip()) ? "computer" : "host " + n.ip())
              + " §7at "
              + run.abs(n.pos()).toShortString());
    }
    for (Scenario.Link l : s.links) {
      say(
          src,
          "§7  cable "
              + l.name()
              + ": "
              + l.a()
              + " "
              + s.eth(l.a(), l.faceA())
              + " <-> "
              + l.b()
              + " "
              + s.eth(l.b(), l.faceB()));
    }
    for (var lead : com.example.evanscomputermod.testing.scenario.RouterScenarios.patchLeads(run))
      say(src, "§7  patch " + lead);
  }

  private static void printWalkthrough(CommandSourceStack src, Scenario s) {
    for (String line : s.walkthrough()) say(src, "§f" + line);
  }

  private static int commands(CommandContext<CommandSourceStack> c) {
    Scenario s = lookup(c);
    if (s == null) return 0;
    say(c.getSource(), "§e" + s.name + "§f: " + s.description);
    for (Scenario.Link l : s.links) {
      say(
          c.getSource(),
          "§7  cable "
              + l.name()
              + ": "
              + l.a()
              + " "
              + s.eth(l.a(), l.faceA())
              + " <-> "
              + l.b()
              + " "
              + s.eth(l.b(), l.faceB()));
    }
    printWalkthrough(c.getSource(), s);
    return 1;
  }

  private static int rerun(CommandContext<CommandSourceStack> c) {
    if (last == null) {
      say(c.getSource(), "§cNothing spawned yet.");
      return 0;
    }
    Active prev = last;
    ACTIVE.remove(prev);
    prev.run().clear();
    start(prev.run().scenario(), c.getSource(), prev.run().origin(), prev.mode());
    return 1;
  }

  private static int status(CommandContext<CommandSourceStack> c) {
    if (ACTIVE.isEmpty()) say(c.getSource(), "§7No scenarios spawned.");
    for (Active a : ACTIVE) {
      ScenarioRun r = a.run();
      say(
          c.getSource(),
          "§f"
              + r.scenario().name
              + " §7at "
              + r.origin().toShortString()
              + ": "
              + r.state()
              + (r.failure() != null ? " - " + r.failure() : ""));
    }
    return ACTIVE.size();
  }

  private static int clear(CommandContext<CommandSourceStack> c) {
    int n = ACTIVE.size();
    for (Active a : ACTIVE) a.run().clear();
    ACTIVE.clear();
    last = null;
    say(c.getSource(), "§7Removed " + n + " scenario(s).");
    return n;
  }

  public static void onServerTick(ServerTickEvent.Post event) {
    for (Active a : ACTIVE) {
      ScenarioRun r = a.run();
      if (r.state() != ScenarioRun.State.RUNNING) continue;
      if (r.tick() == ScenarioRun.State.FAILED) {
        EvansComputerMod.LOGGER.warn("[scenario {}] screens:\n{}", r.scenario().name, r.dump());
        say(a.source(), "§7(screens dumped to the server log)");
      } else if (r.state() == ScenarioRun.State.PASSED && a.mode().equals("manual")) {
        say(a.source(), "§a" + r.scenario().name + ": terminals are up, go ahead.");
      }
    }
  }

  public static void onServerStopping(ServerStoppingEvent event) {
    ACTIVE.clear();
    last = null;
  }

  private ScenarioCommand() {}
}
