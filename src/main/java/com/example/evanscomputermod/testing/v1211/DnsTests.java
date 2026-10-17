package com.example.evanscomputermod.testing.v1211;

//? if <=1.21.1 {
import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.block.TerminalBlockEntity;
import com.example.evanscomputermod.computer.WorldNetwork;
import com.example.evanscomputermod.testing.scenario.DnsScenarios;
import com.example.evanscomputermod.testing.scenario.ScenarioRun;
import com.example.evanscomputermod.worldgen.TechVillageLocator;
import net.minecraft.core.BlockPos;
import net.minecraft.gametest.framework.*;
import net.neoforged.neoforge.gametest.*;

/**
 * DNS (docs/DNS.md): the {@code /ecm scenario} definitions of {@link DnsScenarios} run as
 * tests, one batch each. The ring scenarios run at a house PC of Tech Village 1, placed at its
 * planned site the way {@code /place structure} does (structures are off in the GameTest
 * world), with the ring's infrastructure booted as a world start boots it.
 */
@GameTestHolder("ecm_dns")
@PrefixGameTestTemplate(false)
public final class DnsTests {
  private static final String NS = "ecm_dns";

  private static void run(GameTestHelper h, String name) {
    TestDriver.scenario(h, NS, DnsScenarios.ALL.get(name));
  }

  @GameTest(template = TestDriver.STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = "ecm_dns.lab")
  public static void dns_lab(GameTestHelper h) {
    run(h, "dns_lab");
  }

  @GameTest(template = TestDriver.STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = "ecm_dns.village")
  public static void dns_tech_village(GameTestHelper h) {
    ring(h, "dns_tech_village");
  }

  @GameTest(template = TestDriver.STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = "ecm_dns.resilience")
  public static void dns_resilience(GameTestHelper h) {
    ring(h, "dns_resilience");
  }

  @GameTest(template = TestDriver.STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = "ecm_dns.register")
  public static void dns_register(GameTestHelper h) {
    ring(h, "dns_register");
  }

  @GameTest(template = TestDriver.STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = "ecm_dns.own")
  public static void dns_own_nameserver(GameTestHelper h) {
    tap(h, 3, "dns_own_nameserver", (a, level) -> DnsScenarios.ownNameserver(a, level == null ? null
        : WorldNetwork.get(level).dns(level).resolver(a)));
  }

  @GameTest(template = TestDriver.STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = "ecm_dns.tld")
  public static void dns_tld(GameTestHelper h) {
    tap(h, 4, "dns_tld", (a, level) -> {
      var dns = WorldNetwork.get(level).dns(level);
      return DnsScenarios.tld(a, dns.resolver(a), dns.resolver(a > 5 ? a - 5 : a + 5));
    });
  }

  /**
   * A tap scenario on the real generated ring between village {@code a} and the next (as
   * {@code RouterTests.ringTap}): the ring booted, the span's chunk generated and kept
   * loaded, the layout on top of a span in the middle of the chord.
   */
  static void tap(GameTestHelper h, int a, String name,
                  java.util.function.BiFunction<Integer, net.minecraft.server.level.ServerLevel, com.example.evanscomputermod.testing.scenario.Scenario> make) {
    var level = h.getLevel();
    var d = WorldNetwork.get(level);
    d.plan(level);
    d.applyLinks(level);
    for (var v : d.villages)
      for (String role : d.infrastructure(v.number())) d.boot(level, v.number(), role);
    var ring = d.ring(level);
    int chord = a - 1;
    int[][] path = ring.path(chord);
    int[] mid = path[path.length / 2];
    int cx = mid[0] >> 4, cz = mid[2] >> 4;
    level.getChunk(cx, cz);
    level.setChunkForced(cx, cz, true);
    int i = com.example.evanscomputermod.testing.scenario.RouterScenarios.tapIndex(ring, chord,
        (x, z) -> (x >> 4) == cx && (z >> 4) == cz, pos -> true);
    String[] failure = {null};
    if (i < 0) {
      failure[0] = "no place for the tap layout in chunk " + cx + "," + cz;
      TestDriver.drive(h, NS, name, () -> false, () -> failure[0]);
      return;
    }
    BlockPos span = new BlockPos(path[i][0], path[i][1], path[i][2]);
    var run = new ScenarioRun(make.apply(a, level), level, span.above(),
        msg -> EvansComputerMod.LOGGER.info("[{}] {}", name, msg.replaceAll("§.", "")), 1, true);
    run.build();
    TestDriver.drive(h, NS, name, () -> {
      if (run.tick() != ScenarioRun.State.PASSED) return false;
      run.clear();
      level.setChunkForced(cx, cz, false);
      return true;
    }, () -> {
      if (failure[0] != null) return failure[0];
      if (run.state() != ScenarioRun.State.FAILED) return null;
      failure[0] = run.failure() + "\n" + run.dump() + infrastructure(level);
      run.clear();
      level.setChunkForced(cx, cz, false);
      return failure[0];
    });
  }

  /**
   * Place Tech Village 1, boot the ring (every village's infrastructure, DNS included, and
   * the house router of the PC the scenario uses), then run the scenario at that PC.
   */
  static void ring(GameTestHelper h, String name) {
    var level = h.getLevel();
    var sc = DnsScenarios.ALL.get(name);
    String[] failure = {null};
    ScenarioRun[] run = {null};
    TestDriver.drive(h, NS, name, () -> {
      try {
        if (run[0] == null) {
          var d = WorldNetwork.get(level);
          d.plan(level);
          var site = d.villages.get(0);
          // The ring tests share one world: village 1 is placed and the ring booted once.
          if (placed == null) {
            placed = TechVillageLocator.placeForTest(level, 1);
            TechVillageLocator.hold(level, placed, true);
            d.applyLinks(level);
          }
          for (var v : d.villages)
            for (String role : d.infrastructure(v.number())) d.boot(level, v.number(), role);
          BlockPos pc = DnsScenarios.housePc(level, new BlockPos(site.x(), site.groundY(), site.z()));
          var visit = TechVillageLocator.resolve(level, 1, placed);
          String pcRole = visit.network().terminals().stream().filter(t -> t.pos().equals(pc)).findFirst().orElseThrow().role();
          String routerRole = pcRole.replace(".pc", ".router");
          var router = visit.network().terminals().stream().filter(t -> t.role().equals(routerRole)).findFirst().orElseThrow();
          ((TerminalBlockEntity) level.getBlockEntity(router.pos())).initializeWasm();
          run[0] = new ScenarioRun(sc, level, pc, msg -> EvansComputerMod.LOGGER.info("[{}] {}", name, msg.replaceAll("§.", "")), 1, true);
          run[0].build();
          EvansComputerMod.LOGGER.info("[{}] village 1 placed, ring booted; the PC is {} at {}", name, pcRole, pc);
          return false;
        }
        var st = run[0].tick();
        if (st == ScenarioRun.State.FAILED) failure[0] = run[0].failure() + "\n" + run[0].dump() + infrastructure(level);
        return st == ScenarioRun.State.PASSED;
      } catch (Exception e) {
        failure[0] = e.toString();
        return false;
      }
    }, () -> failure[0]);
  }

  private static net.minecraft.world.level.levelgen.structure.StructureStart placed;

  /** Village 1's DNS-related computers and root letter a's, for a failure report. */
  static String infrastructure(net.minecraft.server.level.ServerLevel level) {
    var d = WorldNetwork.get(level);
    var dns = d.dns(level);
    StringBuilder s = new StringBuilder();
    java.util.List<String> roles = new java.util.ArrayList<>(java.util.List.of(WorldNetwork.ISP_ROUTER, WorldNetwork.WEB));
    roles.addAll(dns.roles(1));
    for (String role : roles)
      s.append("\n--- village 1 ").append(role).append(" ---\n").append(DnsScenarios.infraScreen(level, 1, role).stripTrailing());
    int a = dns.letterVillage('a');
    s.append("\n--- a.root-servers.ring (village ").append(a).append(") ---\n")
        .append(DnsScenarios.infraScreen(level, a, com.example.evanscomputermod.computer.DnsPlan.ROOT).stripTrailing());
    s.append("\n--- IANA (village ").append(dns.iana()).append(") ---\n")
        .append(DnsScenarios.infraScreen(level, dns.iana(), com.example.evanscomputermod.computer.DnsPlan.IANA).stripTrailing());
    return s.toString();
  }
}
//?}
