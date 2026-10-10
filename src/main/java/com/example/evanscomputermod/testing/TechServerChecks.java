package com.example.evanscomputermod.testing;

//? if <=1.21.1 {
import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.block.*;
import com.example.evanscomputermod.computer.*;
import com.example.evanscomputermod.item.ModItems;
import com.example.evanscomputermod.testing.scenario.ScenarioRun;
import com.example.evanscomputermod.worldgen.TechNetworkPiece;
import com.example.evanscomputermod.worldgen.TechVillageLocator;
import com.mojang.brigadier.CommandDispatcher;
import com.mojang.brigadier.arguments.StringArgumentType;
import java.util.*;
import net.minecraft.commands.*;
import net.minecraft.core.*;
import net.minecraft.network.chat.Component;
import net.minecraft.server.MinecraftServer;
import net.minecraft.server.level.*;
import net.minecraft.world.level.ChunkPos;
import net.minecraft.world.level.GameType;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.levelgen.Heightmap;
import net.neoforged.neoforge.event.tick.ServerTickEvent;

/**
 * Tech Village suite, server half (isolated scripted server, fresh world). Natural world
 * generation only: villages, house networks and the fiber ring come from normal chunk
 * generation, never {@code /place}.
 *
 * <ol>
 *   <li>Two neighbouring villages (different vanilla styles when the seed has them): ISP,
 *       mast, patch panel and fiber endpoint at the planned position, house router/PC pairs
 *       with their LAN and buried WAN cables, and the cable segments as seen by the
 *       CableNetworkManager.
 *   <li>A house PC leases an address from its router (whose WAN leased from the ISP over the
 *       physical cable), pings its village server and fetches the neighbour village's page.
 *   <li>The whole chord between the two villages is generated and walked: every path block
 *       is a connected Fiber Span; patch panels at both ends.
 *   <li>Breaking a span cuts that BGP edge and traffic takes the long way round the ring;
 *       replacing it repairs the edge.
 *   <li>Screenshots, driven shot by shot with the hidden client (ECMSHOT messages).
 * </ol>
 *
 * Generating the ~3000-block chord and waiting for BGP convergence make this suite run for
 * several minutes rather than the usual one: the requirement is to walk a complete chord
 * produced by normal generation, which cannot be shortened without testing less.
 */
public final class TechServerChecks {
  private static final org.slf4j.Logger LOG = EvansComputerMod.LOGGER;
  private static final net.minecraft.server.level.TicketType<ChunkPos> WALK =
      net.minecraft.server.level.TicketType.create("ecm_fiber_walk", Comparator.comparingLong(ChunkPos::toLong));
  /** The fiber model fixture near spawn: a six-way cross centred here. */
  public static final BlockPos FIXTURE = new BlockPos(3, 108, 0);

  private static int phase, finishTicks = -1, a, b;
  private static boolean failed;
  private static long phaseStart, nextProbe;
  private static WorldNetwork data;
  private static final Map<Integer, TechVillageLocator.Visit> visits = new HashMap<>();
  private static List<ChunkPos> chordChunks = List.of();
  private static int[][] path;
  private static int cutIndex = -1, terrainIndex = -1, valleyIndex = -1, shortHops, longHops;
  private static int step;
  private static final Deque<Shot> shots = new ArrayDeque<>();
  private static Shot pendingShot;

  private record Shot(String name, Vec feet, float yaw, float pitch, Runnable before) {}

  private record Vec(double x, double y, double z) {}

  public static void register(CommandDispatcher<CommandSourceStack> dispatcher) {
    if (!Boolean.getBoolean("ecm.clientChecks")) return;
    dispatcher.register(
        Commands.literal("ecmvisual")
            .then(
                Commands.literal("shotdone")
                    .then(
                        Commands.argument("name", StringArgumentType.word())
                            .executes(
                                c -> {
                                  String name = StringArgumentType.getString(c, "name");
                                  if (pendingShot != null && pendingShot.name.equals(name)) pendingShot = null;
                                  return 1;
                                }))));
  }

  private static void check(boolean condition, String message) {
    if (!condition) throw new IllegalStateException(message);
  }

  private static void pass(String name, String detail) {
    LOG.info("ECM_VISUAL_SERVER_PASS {} {}", name, detail);
  }

  private static void next(int p) {
    phase = p;
    phaseStart = System.currentTimeMillis();
    nextProbe = 0;
    step = 0;
  }

  private static long elapsed() {
    return System.currentTimeMillis() - phaseStart;
  }

  public static void tick(ServerTickEvent.Post event) {
    if (finishTicks >= 0) {
      if (--finishTicks == 0) event.getServer().halt(false);
      return;
    }
    if (!Boolean.getBoolean("ecm.clientChecks") || failed) return;
    var server = event.getServer();
    if (server.getPlayerList().getPlayers().isEmpty()) return;
    var level = server.overworld();
    var player = server.getPlayerList().getPlayers().get(0);
    try {
      switch (phase) {
        case 0 -> setup(server, level, player);
        case 1 -> villages(level);
        case 2 -> houseNetwork(level);
        case 3 -> loadChord(level);
        case 4 -> walkChord(level);
        case 5 -> cutAndReroute(level);
        case 6 -> repair(level);
        case 7 -> screenshots(server, level, player);
        case 8 -> finish(server, player);
        default -> {}
      }
    } catch (Throwable t) {
      failed = true;
      LOG.error("ECM_VISUAL_SERVER_FAIL phase " + phase, t);
      server.halt(false);
    }
  }

  // ------------------------------------------------------------------ 0: setup

  private static void setup(MinecraftServer server, ServerLevel level, ServerPlayer player) {
    server.getPlayerList().op(player.getGameProfile());
    player.setGameMode(GameType.CREATIVE);
    player.getAbilities().flying = true;
    player.onUpdateAbilities();
    data = WorldNetwork.get(level);
    data.plan(level);
    a = 1;
    for (int i = 1; i <= 10; i++) {
      int n = i == 10 ? 1 : i + 1;
      if (!data.villages.get(i - 1).style().equals(data.villages.get(n - 1).style())) {
        a = i;
        break;
      }
    }
    b = a == 10 ? 1 : a + 1;
    boolean flat = level.getChunkSource().getGenerator() instanceof net.minecraft.world.level.levelgen.FlatLevelSource;
    List<String> cases = new ArrayList<>(List.of("fiber_connected", "fiber_disconnected", "fiber_repaired"));
    cases.add("isp_" + style(a));
    if (!style(b).equals(style(a))) cases.add("isp_" + style(b));
    cases.addAll(List.of("house_interior", "fiber_mast"));
    if (!flat) cases.addAll(List.of("fiber_terrain", "fiber_valley"));
    cases.add("village_arrival");
    List<String> serverOnly =
        new ArrayList<>(List.of("natural_village_" + a, "natural_village_" + b, "house_network", "fiber_chord",
            "fiber_cut_reroute", "fiber_repair", "scenario_commands"));
    LOG.info("ECM_VISUAL_TECH_CASES {}", String.join(",", cases));
    LOG.info("ECM_VISUAL_TECH_SERVER {}", String.join(",", serverOnly));
    LOG.info("Tech checks: villages {} ({}) and {} ({}); seed {}", a, style(a), b, style(b), level.getSeed());
    next(1);
  }

  private static String style(int n) {
    return data.villages.get(n - 1).style();
  }

  // ------------------------------------------------------------------ 1: villages

  private static void villages(ServerLevel level) {
    for (int n : new int[] {a, b}) {
      long t0 = System.currentTimeMillis();
      var visit = TechVillageLocator.visit(level, n);
      visits.put(n, visit);
      // Keep the village loaded: its computers run only while their chunks are loaded.
      TechVillageLocator.hold(level, visit.start(), true);
      var site = data.villages.get(n - 1);
      check(
          level.getBlockState(site.patchPanel()).is(ModBlocks.FIBER_PATCH_PANEL.get()),
          "village " + n + ": patch panel not at the planned " + site.patchPanel().toShortString()
              + " (found " + level.getBlockState(site.patchPanel()) + ")");
      var end = level.getBlockState(site.endpoint());
      check(
          end.is(ModBlocks.FIBER_SPAN.get()) && end.getValue(NetworkCableBlock.DOWN),
          "village " + n + ": fiber endpoint above the panel is " + end);
      var routerLocal = new BlockPos(8, 1, 6);
      var router = TechVillageLocator.world(visit.isp(), routerLocal);
      var server = TechVillageLocator.world(visit.isp(), new BlockPos(10, 1, 6));
      check(terminalId(level, router).equals(data.identity(level, n, "isp.router")), "ISP router identity at " + router);
      check(terminalId(level, server).equals(data.identity(level, n, "isp.server")), "ISP server identity at " + server);
      check(visit.network() != null, "village " + n + " has no network piece");
      var terminals = visit.network().terminals();
      int houses = terminals.size() / 2;
      check(houses >= 2, "village " + n + " has only " + houses + " networked houses");
      for (var t : terminals)
        check(terminalId(level, t.pos()).equals(data.identity(level, n, t.role())),
            "village " + n + " " + t.role() + " missing at " + t.pos().toShortString());
      check(visit.computers().size() == 2 + terminals.size(),
          "village " + n + " has " + visit.computers().size() + " computers, expected " + (2 + terminals.size()));
      // Physical cabling: every house WAN reaches the ISP's access cable; LAN pairs are isolated.
      Set<BlockPos> wan = cableComponent(level, router.below());
      check(!wan.isEmpty(), "no access cable under the ISP router");
      for (int h = 1; h <= houses; h++) {
        BlockPos hr = find(terminals, "house" + h + ".router"), pc = find(terminals, "house" + h + ".pc");
        check(wan.contains(hr.below()), "house " + h + " WAN cable does not reach the ISP");
        Set<BlockPos> lan = cableComponent(level, hr.above());
        check(lan.equals(Set.of(hr.above(), pc.above())),
            "house " + h + " LAN cable touches other cable: " + lan.size() + " blocks");
      }
      check(!wan.contains(router.above()) && !wan.contains(server.above()), "access cable touches the server LAN");
      pass("natural_village_" + n,
          "style=" + style(n) + " houses=" + houses + " computers=" + visit.computers().size()
              + " accessCable=" + wan.size() + " panel=" + site.patchPanel().toShortString()
              + " ms=" + (System.currentTimeMillis() - t0));
    }
    next(2);
  }

  private static BlockPos find(List<TechNetworkPiece.Terminal> terminals, String role) {
    return terminals.stream().filter(t -> t.role().equals(role)).findFirst().orElseThrow().pos();
  }

  private static UUID terminalId(ServerLevel level, BlockPos pos) {
    return level.getBlockEntity(pos) instanceof TerminalBlockEntity t ? t.getComputerId() : new UUID(0, 0);
  }

  /** Face-connected NetworkCableBlocks reachable from {@code start} (loaded chunks only). */
  public static Set<BlockPos> cableComponent(ServerLevel level, BlockPos start) {
    Set<BlockPos> seen = new HashSet<>();
    if (!(level.getBlockState(start).getBlock() instanceof NetworkCableBlock)) return seen;
    Deque<BlockPos> queue = new ArrayDeque<>(List.of(start));
    seen.add(start);
    while (!queue.isEmpty() && seen.size() < 20000) {
      BlockPos p = queue.poll();
      for (Direction d : Direction.values()) {
        BlockPos n = p.relative(d);
        if (!seen.contains(n) && level.getBlockState(n).getBlock() instanceof NetworkCableBlock) {
          seen.add(n);
          queue.add(n);
        }
      }
    }
    return seen;
  }

  // ------------------------------------------------------------------ 2: house PC end to end

  private static TerminalBlockEntity pc(ServerLevel level) {
    var pos = find(visits.get(a).network().terminals(), "house1.pc");
    return (TerminalBlockEntity) level.getBlockEntity(pos);
  }

  private static String screen(TerminalBlockEntity be) {
    return ScenarioRun.screen(be.getDisplay());
  }

  private static void send(TerminalBlockEntity be, String command) {
    be.getComputer().sendInput(command + "\n");
  }

  private static void houseNetwork(ServerLevel level) {
    var pc = pc(level);
    // The structure's computers boot on chunk load; the CableNetworkManager must agree
    // with the physical cabling once they register.
    if (pc.getComputer() == null || !screen(pc).contains("/ >")) {
      check(elapsed() < 60_000, "house PC did not boot within 60 s:\n" + screen(pc));
      return;
    }
    var mgr = CableNetworkManager.getInstance();
    var terminals = visits.get(a).network().terminals();
    byte[] ispAccess = NetworkHub.deriveMac(data.identity(level, a, "isp.router"), 0);
    byte[] houseWan = NetworkHub.deriveMac(data.identity(level, a, "house1.router"), 0);
    byte[] houseLan = NetworkHub.deriveMac(data.identity(level, a, "house1.router"), 1);
    byte[] pcNic = NetworkHub.deriveMac(data.identity(level, a, "house1.pc"), 1);
    if (step == 0) {
      if (!(mgr.areOnSameNetwork(ispAccess, houseWan) && mgr.areOnSameNetwork(houseLan, pcNic))) {
        check(elapsed() < 60_000, "house router never joined the ISP access segment / PC LAN");
        return;
      }
      check(!mgr.areOnSameNetwork(houseWan, houseLan), "house WAN and LAN share a segment");
      check(!mgr.areOnSameNetwork(ispAccess, pcNic), "PC reaches the access segment without NAT");
      step = 1;
    }
    String target = "100." + (64 + a) + ".0.10";
    String remote = "100." + (64 + b) + ".0.10";
    if (step == 1) {
      if (screen(pc).contains("1 packets sent, 1 received")) {
        step = 2;
        nextProbe = 0;
      } else if (System.currentTimeMillis() > nextProbe) {
        send(pc, "ping " + target + " -n 1");
        nextProbe = System.currentTimeMillis() + 6000;
      }
    }
    if (step == 2) {
      if (screen(pc).contains("Tech Village " + b)) {
        pass("house_network",
            "village=" + a + " pc=" + terminals.stream().filter(t -> t.role().equals("house1.pc")).findFirst().get().pos().toShortString()
                + " pinged " + target + " and fetched " + remote + " ms=" + elapsed());
        next(3);
        return;
      }
      if (System.currentTimeMillis() > nextProbe) {
        send(pc, "curl http://" + remote + "/index.html");
        nextProbe = System.currentTimeMillis() + 8000;
      }
    }
    check(elapsed() < 120_000,
        "house PC could not reach the servers (step " + step + "):\n" + screen(pc).stripTrailing());
  }

  // ------------------------------------------------------------------ 3/4: whole chord

  private static void loadChord(ServerLevel level) {
    if (step == 0) {
      var ring = data.ring(level);
      path = ring.path(WorldNetwork.chord(a, b));
      if (!Arrays.equals(path[0], new int[] {data.villages.get(a - 1).x(), data.villages.get(a - 1).endY(), data.villages.get(a - 1).z()}))
        path = reverse(path);
      Set<Long> seen = new LinkedHashSet<>();
      for (int[] p : path) seen.add(ChunkPos.asLong(p[0] >> 4, p[2] >> 4));
      chordChunks = seen.stream().map(ChunkPos::new).toList();
      for (var c : chordChunks) level.getChunkSource().addRegionTicket(WALK, c, 0, c);
      LOG.info("Fiber walk: {} blocks across {} chunks", path.length, chordChunks.size());
      step = 1;
    }
    long loaded = chordChunks.stream().filter(c -> level.hasChunk(c.x, c.z)).count();
    if (loaded == chordChunks.size()) {
      next(4);
      return;
    }
    check(elapsed() < 300_000, "chord chunks still generating: " + loaded + "/" + chordChunks.size());
  }

  private static int[][] reverse(int[][] p) {
    int[][] r = new int[p.length][];
    for (int i = 0; i < p.length; i++) r[i] = p[p.length - 1 - i];
    return r;
  }

  private static BlockPos at(int i) {
    return new BlockPos(path[i][0], path[i][1], path[i][2]);
  }

  private static void walkChord(ServerLevel level) {
    check(FiberLine.faceConnected(path), "chord path is not face-connected");
    int buried = 0, floating = 0, maxGap = 0;
    for (int i = 0; i < path.length; i++) {
      BlockPos p = at(i);
      var s = level.getBlockState(p);
      check(s.is(ModBlocks.FIBER_SPAN.get()), "path block " + i + "/" + path.length + " at " + p.toShortString() + " is " + s);
      for (int j : new int[] {i - 1, i + 1}) {
        if (j < 0 || j >= path.length) continue;
        Direction d = Direction.fromDelta(path[j][0] - path[i][0], path[j][1] - path[i][1], path[j][2] - path[i][2]);
        check(s.getValue(NetworkCableBlock.getPropertyForDirection(d)), "span " + i + " has no arm toward " + d);
      }
      int ground = level.getHeight(Heightmap.Types.MOTION_BLOCKING_NO_LEAVES, p.getX(), p.getZ());
      if (ground > p.getY() + 1) {
        buried++;
        if (terrainIndex < 0 && i > 200 && i < path.length - 200 && ground > p.getY() + 4) terrainIndex = i;
      }
      int gap = p.getY() - ground;
      if (gap > 0) floating++;
      if (gap > maxGap && i > 200 && i < path.length - 200) {
        maxGap = gap;
        valleyIndex = i;
      }
    }
    for (int n : new int[] {a, b})
      check(level.getBlockState(data.villages.get(n - 1).patchPanel()).is(ModBlocks.FIBER_PATCH_PANEL.get()),
          "missing patch panel at village " + n);
    // A carve-through view: step back to where the line enters the hillside.
    if (terrainIndex > 0)
      while (terrainIndex > 1
          && level.getHeight(Heightmap.Types.MOTION_BLOCKING_NO_LEAVES, path[terrainIndex - 1][0], path[terrainIndex - 1][2])
              > path[terrainIndex - 1][1] + 1) terrainIndex--;
    cutIndex = path.length / 2;
    pass("fiber_chord",
        "chord=" + WorldNetwork.linkName(a, b) + " blocks=" + path.length + " chunks=" + chordChunks.size()
            + " buried=" + buried + " floating=" + floating + " maxGap=" + maxGap + " ms=" + elapsed());
    next(5);
  }

  // ------------------------------------------------------------------ 5/6: cut and repair

  /** Hop count of the last traceroute that reached {@code target}, or -1 (running / no reply). */
  private static int hops(String screen, String target) {
    int at = screen.lastIndexOf("traceroute to " + target);
    if (at < 0) return -1;
    var m = java.util.regex.Pattern.compile("(?m)^\\s*(\\d+)\\s+" + java.util.regex.Pattern.quote(target) + "\\s*$")
        .matcher(screen.substring(at));
    return m.find() ? Integer.parseInt(m.group(1)) : -1;
  }

  /** Step machine: traceroute the remote server until it completes with a reply. */
  private static int traceroute(TerminalBlockEntity pc, String remote) {
    if (System.currentTimeMillis() > nextProbe) {
      send(pc, "clear");
      send(pc, "traceroute " + remote + " 16");
      nextProbe = System.currentTimeMillis() + 20_000;
      return -1;
    }
    int h = hops(screen(pc), remote);
    return h > 0 ? h : -1;
  }

  private static void cutAndReroute(ServerLevel level) {
    var pc = pc(level);
    String remote = "100." + (64 + b) + ".0.10";
    var mgr = CableNetworkManager.getInstance();
    byte[] fiber = NetworkHub.deriveMac(data.identity(level, a, "isp.router"), WorldNetwork.FIBER_NEXT_PORT);
    if (step == 0) {
      int h = traceroute(pc, remote);
      if (h > 0) {
        shortHops = h;
        step = 1;
      }
    } else if (step == 1) {
      check(data.linkUp(level, a, b) && mgr.carrierOf(fiber), "fiber link down before the cut");
      level.destroyBlock(at(cutIndex), false);
      check(!data.linkUp(level, a, b), "breaking a span did not cut " + WorldNetwork.linkName(a, b));
      check(!mgr.carrierOf(fiber), "router fiber NIC kept carrier after the cut");
      int other = b == 10 ? 1 : b + 1;
      check(data.linkUp(level, b, other), "control: the next chord must stay intact");
      // Off-path control: a span removed beside the path changes nothing.
      BlockPos beside = at(cutIndex).above(3);
      level.setBlock(beside, ModBlocks.FIBER_SPAN.get().defaultBlockState(), 3);
      level.destroyBlock(beside, false);
      check(data.brokenFiber.size() == 1, "off-path span changed the ring");
      nextProbe = 0;
      step = 2;
    } else {
      int h = traceroute(pc, remote);
      if (h > 0 && h >= shortHops + 5) {
        longHops = h;
        pass("fiber_cut_reroute",
            "cut " + at(cutIndex).toShortString() + " hops " + shortHops + " -> " + longHops + " ms=" + elapsed());
        next(6);
        return;
      }
    }
    check(elapsed() < 120_000, "reroute after the fiber cut failed (step " + step + ", short hops " + shortHops
        + "):\n" + screen(pc).stripTrailing());
  }

  private static void repair(ServerLevel level) {
    var pc = pc(level);
    String remote = "100." + (64 + b) + ".0.10";
    if (step == 0) {
      // Placed the way a player places it: onPlace runs and repairs the chord.
      level.setBlock(at(cutIndex), ModBlocks.FIBER_SPAN.get().defaultBlockState(), 3);
      check(data.linkUp(level, a, b), "replacing the span did not repair " + WorldNetwork.linkName(a, b));
      var s = level.getBlockState(at(cutIndex));
      check(s.getValue(NetworkCableBlock.getPropertyForDirection(Direction.fromDelta(
              path[cutIndex - 1][0] - path[cutIndex][0], path[cutIndex - 1][1] - path[cutIndex][1],
              path[cutIndex - 1][2] - path[cutIndex][2]))), "replaced span did not rejoin its neighbour");
      nextProbe = 0;
      step = 1;
      return;
    }
    int h = traceroute(pc, remote);
    if (h > 0 && h == shortHops) {
      pass("fiber_repair", "hops back to " + h + " ms=" + elapsed());
      for (var c : chordChunks) level.getChunkSource().removeRegionTicket(WALK, c, 0, c);
      next(7);
      return;
    }
    check(elapsed() < 120_000, "route did not return after repair:\n" + screen(pc).stripTrailing());
  }

  // ------------------------------------------------------------------ 7: screenshots

  private static Vec feet(BlockPos target, Direction side, int distance, int up) {
    BlockPos p = target.relative(side, distance).above(up);
    return new Vec(p.getX() + 0.5, p.getY(), p.getZ() + 0.5);
  }

  /** Yaw/pitch that look from an eye at {@code feet} to the centre of {@code target}. */
  private static float[] look(Vec feet, BlockPos target) {
    double dx = target.getX() + 0.5 - feet.x, dy = target.getY() + 0.5 - (feet.y + 1.62), dz = target.getZ() + 0.5 - feet.z;
    float yaw = (float) (Math.toDegrees(Math.atan2(dz, dx)) - 90);
    float pitch = (float) -Math.toDegrees(Math.atan2(dy, Math.sqrt(dx * dx + dz * dz)));
    return new float[] {yaw, pitch};
  }

  private static Shot shot(String name, Vec feet, BlockPos target, Runnable before) {
    float[] l = look(feet, target);
    return new Shot(name, feet, l[0], l[1], before);
  }

  private static void buildFixture(ServerLevel level) {
    for (int x = -5; x <= 16; x++)
      for (int z = -4; z <= 16; z++)
        level.setBlock(new BlockPos(x, 103, z), Blocks.SMOOTH_STONE.defaultBlockState(), 3);
    for (int x = 0; x <= 9; x++) level.setBlock(new BlockPos(x, 108, 0), ModBlocks.FIBER_SPAN.get().defaultBlockState(), 3);
    level.setBlock(FIXTURE.south(), ModBlocks.FIBER_SPAN.get().defaultBlockState(), 3);
    level.setBlock(FIXTURE.above(), ModBlocks.FIBER_SPAN.get().defaultBlockState(), 3);
    // Unsupported riser from a patch panel (fiber needs no poles).
    for (int y = 104; y < 108; y++) level.setBlock(new BlockPos(0, y, 0), ModBlocks.FIBER_SPAN.get().defaultBlockState(), 3);
    level.setBlock(new BlockPos(0, 104, 0).below(), ModBlocks.FIBER_PATCH_PANEL.get().defaultBlockState()
        .setValue(FiberPatchPanelBlock.FACING, Direction.SOUTH), 3);
    var frame = new net.minecraft.world.entity.decoration.ItemFrame(level, new BlockPos(12, 106, 0), Direction.SOUTH);
    level.setBlock(new BlockPos(12, 106, -1), Blocks.SMOOTH_STONE.defaultBlockState(), 3);
    frame.setItem(ModItems.ALWAYS_ON_MODULE.get().getDefaultInstance());
    level.addFreshEntity(frame);
    var middle = level.getBlockState(FIXTURE);
    check(middle.getValue(NetworkCableBlock.EAST) && middle.getValue(NetworkCableBlock.WEST)
        && middle.getValue(NetworkCableBlock.SOUTH) && middle.getValue(NetworkCableBlock.UP), "neighbor state did not connect");
    check(!level.getBlockState(new BlockPos(9, 108, 0)).getValue(NetworkCableBlock.EAST), "isolated east end rendered an arm");
    check(level.getBlockState(new BlockPos(0, 104, 0)).getValue(NetworkCableBlock.DOWN), "riser did not join its patch panel");
  }

  private static void planShots(ServerLevel level) {
    Vec fixtureEye = new Vec(6.5, 104, 9.5);
    shots.add(new Shot("fiber_connected", fixtureEye, 180, -5, () -> {
      buildFixture(level);
      pass("fiber_connected", "");
    }));
    shots.add(new Shot("fiber_disconnected", fixtureEye, 180, -5, () -> {
      level.removeBlock(FIXTURE.east(), false);
      check(!level.getBlockState(FIXTURE).getValue(NetworkCableBlock.EAST), "east arm did not disconnect");
      pass("fiber_disconnected", "");
    }));
    shots.add(new Shot("fiber_repaired", fixtureEye, 180, -5, () -> {
      level.setBlock(FIXTURE.east(), ModBlocks.FIBER_SPAN.get().defaultBlockState(), 3);
      check(level.getBlockState(FIXTURE).getValue(NetworkCableBlock.EAST), "east arm did not repair");
      pass("fiber_repaired", "");
    }));
    Set<String> styles = new HashSet<>();
    for (int n : new int[] {a, b}) {
      if (!styles.add(style(n))) continue;
      var visit = visits.get(n);
      var site = data.villages.get(n - 1);
      Direction front = visit.isp().getRotation().rotate(Direction.SOUTH);
      Direction right = front.getCounterClockWise();
      BlockPos mast = new BlockPos(site.x(), site.groundY() + 4, site.z());
      Vec eye = feet(mast.relative(right, 9), front, 17, 5);
      String name = "isp_" + style(n);
      shots.add(shot(name, eye, mast, () -> pass(name, "village=" + n)));
    }
    // House interior: stand in front of house 1's desk in village a.
    var t = visits.get(a).network().terminals().stream().filter(x -> x.role().equals("house1.router")).findFirst().orElseThrow();
    BlockPos desk = t.pos();
    int back = level.getBlockState(desk.relative(t.facing(), 2)).isAir() && level.getBlockState(desk.relative(t.facing(), 2).above()).isAir() ? 2 : 1;
    Vec houseEye = feet(desk, t.facing(), back, 0);
    shots.add(shot("house_interior", houseEye, desk.below(), () -> pass("house_interior", "desk=" + desk.toShortString())));
    // The fiber leaving the mast top.
    var site = data.villages.get(a - 1);
    int[] toward = path[Math.min(40, path.length - 1)];
    Direction along = Direction.getNearest(toward[0] - site.x(), 0, toward[2] - site.z());
    BlockPos end = site.endpoint().relative(along, 6);
    Vec mastEye = feet(end, along.getClockWise(), 14, 2);
    shots.add(shot("fiber_mast", mastEye, end.relative(along.getOpposite(), 3), () -> pass("fiber_mast", "endpoint=" + site.endpoint().toShortString())));
    boolean flat = level.getChunkSource().getGenerator() instanceof net.minecraft.world.level.levelgen.FlatLevelSource;
    if (!flat) {
      int ti = terrainIndex > 0 ? terrainIndex : path.length / 3;
      BlockPos entry = at(ti);
      Direction dir = Direction.getNearest(path[ti + 1][0] - path[ti - 1][0], 0, path[ti + 1][2] - path[ti - 1][2]);
      Vec terrainEye = feet(entry.relative(dir.getOpposite(), 6), dir.getClockWise(), 12, 3);
      shots.add(shot("fiber_terrain", terrainEye, entry, () -> pass("fiber_terrain", "entry=" + entry.toShortString() + " index=" + ti)));
      int vi = valleyIndex > 0 ? valleyIndex : path.length / 2;
      BlockPos span = at(vi);
      Direction vdir = Direction.getNearest(path[vi + 1][0] - path[vi - 1][0], 0, path[vi + 1][2] - path[vi - 1][2]);
      int gap = span.getY() - level.getHeight(Heightmap.Types.MOTION_BLOCKING_NO_LEAVES, span.getX(), span.getZ());
      Vec valleyEye = feet(span, vdir.getClockWise(), 22, -Math.min(6, gap / 3));
      shots.add(shot("fiber_valley", valleyEye, span, () -> pass("fiber_valley", "span=" + span.toShortString() + " gap=" + gap)));
    }
    shots.add(new Shot("village_arrival", null, 0, 0, null));
  }

  private static void screenshots(MinecraftServer server, ServerLevel level, ServerPlayer player) throws Exception {
    if (step == 0) {
      planShots(level);
      step = 1;
    }
    if (pendingShot != null) {
      check(elapsed() < 90_000, "client did not capture " + pendingShot.name);
      return;
    }
    Shot s = shots.poll();
    if (s == null) {
      next(8);
      return;
    }
    phaseStart = System.currentTimeMillis();
    Vec feet;
    if (s.name.equals("village_arrival")) {
      var visit = visits.get(a);
      int result = server.getCommands().getDispatcher().execute("ecm techvillage tp " + a, player.createCommandSourceStack());
      check(result > 0, "village teleport command failed");
      check(player.position().distanceTo(net.minecraft.world.phys.Vec3.atBottomCenterOf(visit.arrival())) < 2,
          "village command landed at " + player.position() + " instead of " + visit.arrival());
      player.setXRot(0);
      feet = new Vec(player.getX(), player.getY(), player.getZ());
      pass("village_arrival", "arrival=" + visit.arrival().toShortString());
    } else {
      if (s.before != null) s.before.run();
      player.teleportTo(level, s.feet.x, s.feet.y, s.feet.z, s.yaw, s.pitch);
      feet = s.feet;
    }
    pendingShot = s;
    player.sendSystemMessage(Component.literal(
        "ECMSHOT " + s.name + " " + feet.x + " " + feet.y + " " + feet.z));
  }

  // ------------------------------------------------------------------ 8: finish

  private static void finish(MinecraftServer server, ServerPlayer player) throws Exception {
    var commands = server.getCommands();
    var source = player.createCommandSourceStack();
    for (String command :
        new String[] {
          "ecm techvillage list",
          "ecm techvillage info " + a,
          "ecm net links",
          "ecm scenario commands router_home",
          "ecm scenario spawn router_bgp_pair manual",
          "ecm scenario links",
          "ecm scenario link ring1 down",
          "ecm scenario link ring1 up",
          "ecm scenario clear"
        })
      check(commands.getDispatcher().execute(command, source) > 0, "Command failed: " + command);
    pass("scenario_commands", "");
    player.sendSystemMessage(Component.literal("ECMSHOT_END"));
    // Give freshly cleared fixture chunks and their asynchronous IO time to settle
    // before the server begins its unload loop.
    finishTicks = 100;
    next(9);
  }
}
//?}
