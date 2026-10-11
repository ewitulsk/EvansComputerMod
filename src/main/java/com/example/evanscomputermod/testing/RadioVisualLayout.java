package com.example.evanscomputermod.testing;

//? if <=1.21.1 {
import com.example.evanscomputermod.EvansComputerMod;
import java.util.*;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.resources.ResourceLocation;
import net.minecraft.world.item.BlockItem;
import net.minecraft.world.item.Item;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.block.state.properties.BlockStateProperties;
import net.minecraft.world.level.block.state.properties.Property;
import net.minecraft.world.phys.Vec3;

/**
 * The radio render suite's display, computed identically on the scripted server
 * (which builds it) and the hidden client (which frames and verifies it) from the
 * synced registries. Data-driven: {@link #SPECS} lists block ids and states in
 * display order; ids that are not registered are skipped (and logged), and any
 * other block or item whose class lives in the {@code radio} package is appended
 * automatically, so new radio content is rendered without editing this file.
 */
public final class RadioVisualLayout {
  public static final String SUITE_PROPERTY = "ecm.clientChecks.suite";
  /** Row height: the floor is one block below. High enough to clear normal terrain. */
  public static final int Y = 170;
  public static final int Z = 0;
  public static final int SPACING = 2;
  /** Item frames hang on a wall west of the block row. */
  public static final int ITEM_COLUMNS = 6;
  public static final int ITEM_X0 = -3 - ITEM_COLUMNS;

  public static final String OVERVIEW = "radio_overview";
  public static final String ITEMS = "radio_items";

  /** Display groups, in row order; each non-empty group gets a radio_closeup_<group> case. */
  public static final List<String> GROUPS =
      List.of("power", "wifi", "antenna", "dish", "amplifier", "misc");

  /** How the server prepares a placed block so the listed state is the steady state. */
  public enum Setup {
    NONE,
    /** Burner: put coal in the fuel slot so it really burns (lit=true holds). */
    FUEL,
    /** Access point: stand it on a network_cable so its port goes active. */
    CABLE_BELOW
  }

  public record Spec(String group, String id, String props, Setup setup) {}

  /** One block in the row. {@code expected} holds only the properties the layout set. */
  public record Placed(
      String group,
      ResourceLocation id,
      BlockState state,
      Map<Property<?>, Comparable<?>> expected,
      BlockPos pos,
      Setup setup) {
    public String label() {
      var p = new StringBuilder(id.getPath());
      expected.forEach((k, v) -> p.append(',').append(k.getName()).append('=').append(v));
      return p.toString();
    }

    /** True when {@code actual} is this block with every property the layout set. */
    public boolean matches(BlockState actual) {
      if (actual.getBlock() != state.getBlock()) return false;
      for (var e : expected.entrySet())
        if (!actual.getValue(e.getKey()).equals(e.getValue())) return false;
      return true;
    }
  }

  public record View(String name, Vec3 feet, float yaw, float pitch, List<Placed> blocks) {}

  public record Layout(
      List<Placed> blocks, List<ResourceLocation> items, List<String> skipped, List<View> views) {
    /** The fixtures' origin: east of the row's last block, clear of its views. */
    public static int fixtureX0(int rowEnd) {
      return rowEnd + 4;
    }

    public View view(String name) {
      for (var v : views) if (v.name().equals(name)) return v;
      throw new IllegalArgumentException("Unknown radio view " + name);
    }

    public BlockPos framePos(int index) {
      return new BlockPos(itemX0() + index % ITEM_COLUMNS, Y + index / ITEM_COLUMNS, Z);
    }

    public int itemX0() {
      return ITEM_X0;
    }

    public int rowEnd() {
      return blocks.isEmpty() ? 0 : blocks.get(blocks.size() - 1).pos().getX();
    }
  }

  /**
   * Blocks in display order. Later radio lanes add antennas, dishes and amplifiers;
   * their likely ids are listed so they show up the moment they are registered.
   */
  static final List<Spec> SPECS =
      List.of(
          new Spec("power", "burner_generator", "lit=false", Setup.NONE),
          new Spec("power", "burner_generator", "lit=true", Setup.FUEL),
          new Spec("power", "sdr_basic", "", Setup.NONE),
          new Spec("power", "sdr_standard", "", Setup.NONE),
          new Spec("power", "sdr_advanced", "", Setup.NONE),
          new Spec("wifi", "access_point", "active=false", Setup.NONE),
          new Spec("wifi", "access_point", "active=true", Setup.CABLE_BELOW),
          new Spec("antenna", "copper_wire", "", Setup.NONE),
          new Spec("antenna", "antenna_mast", "", Setup.NONE),
          new Spec("antenna", "dipole_antenna", "", Setup.NONE),
          new Spec("antenna", "yagi_antenna", "", Setup.NONE),
          new Spec("antenna", "whip_antenna", "", Setup.NONE),
          new Spec("dish", "dish", "", Setup.NONE),
          new Spec("dish", "satellite_dish", "", Setup.NONE),
          new Spec("dish", "parabolic_dish", "", Setup.NONE),
          new Spec("amplifier", "amplifier", "", Setup.NONE),
          new Spec("amplifier", "rf_amplifier", "", Setup.NONE),
          new Spec("amplifier", "power_amplifier", "", Setup.NONE),
          new Spec("amplifier", "low_noise_amplifier", "", Setup.NONE));

  /**
   * Items shown in frames besides the block items of the row: the radio items, then the
   * network items whose models were redesigned alongside the radio ones (shown next to the
   * Terminal and Screen for comparison).
   */
  static final List<String> ITEM_SPECS =
      List.of(
          "handheld_radio",
          "controller_receiver_module",
          "interface_probe",
          "fiber_span",
          "network_cable",
          "interface_block",
          "fiber_patch_panel",
          "screen_block",
          "terminal_block");

  /** A connection-showcase block: id, position relative to the fixtures' origin, and its properties. */
  record FixtureBlock(String id, int dx, int dy, int dz, String props) {}

  /** A close-up of several blocks that are only meaningful together (a jack with its coax, a Screen cluster). */
  record Fixture(String name, List<FixtureBlock> blocks, double[] eye, double[] target) {}

  private static final String NONE6 = "north=false,south=false,east=false,west=false,up=false,down=false";

  /** {@code arms} set to true over all six arms false (later keys win). */
  private static String arms(String arms) {
    var m = new LinkedHashMap<String, String>();
    for (var part : (NONE6 + "," + arms).split(",")) {
      var kv = part.split("=", 2);
      m.put(kv[0], kv[1]);
    }
    var out = new StringBuilder();
    m.forEach((k, v) -> out.append(out.length() == 0 ? "" : ",").append(k).append('=').append(v));
    return out.toString();
  }

  /**
   * Close-ups placed east of the row. Coax, fiber and cable states list the arms the real
   * connection rules must produce, so the paired assertions also check the connections.
   */
  static final List<Fixture> FIXTURES =
      List.of(
          // The SDR's back-face antenna jack (Standard, back to the camera), a side socket (Basic)
          // and the top socket (Advanced, front to the camera), each with coax plugged in.
          new Fixture(
              "radio_closeup_sdr_coax",
              List.of(
                  new FixtureBlock("sdr_standard", 0, 0, 0, "facing=north"),
                  new FixtureBlock("coax_cable", 0, 0, 1, arms("north=true,east=true")),
                  new FixtureBlock("coax_cable", 1, 0, 1, arms("west=true")),
                  new FixtureBlock("sdr_basic", 3, 0, 0, "facing=north"),
                  new FixtureBlock("coax_cable", 4, 0, 0, arms("west=true,south=true")),
                  new FixtureBlock("coax_cable", 4, 0, 1, arms("north=true")),
                  new FixtureBlock("sdr_advanced", 6, 0, 0, "facing=south"),
                  new FixtureBlock("coax_cable", 6, 1, 0, arms("down=true,up=true")),
                  new FixtureBlock("coax_cable", 6, 2, 0, arms("down=true"))),
              new double[] {3.5, 2.3, 3.7},
              new double[] {3.5, 0.8, 0.5}),
          // The tuner with coax on its TX jack (back) and its ANT feedthrough (top).
          new Fixture(
              "radio_closeup_tuner_coax",
              List.of(
                  new FixtureBlock("antenna_tuner", 9, 0, 0, "facing=north"),
                  new FixtureBlock("coax_cable", 9, 0, 1, arms("north=true,east=true")),
                  new FixtureBlock("coax_cable", 10, 0, 1, arms("west=true")),
                  new FixtureBlock("coax_cable", 9, 1, 0, arms("down=true,up=true")),
                  new FixtureBlock("coax_cable", 9, 2, 0, arms("down=true"))),
              new double[] {11.4, 2.7, 3.4},
              new double[] {9.5, 1.0, 0.6}),
          // Patch panels: fiber in on one side and copper on the other (front to the camera),
          // and one turned round to show the copper side with a cable in its back gland.
          new Fixture(
              "tech_closeup_patch_panel",
              List.of(
                  new FixtureBlock("fiber_patch_panel", 13, 0, 0, "facing=south"),
                  new FixtureBlock("fiber_span", 12, 0, 0, arms("east=true,up=true")),
                  new FixtureBlock("fiber_span", 12, 1, 0, arms("down=true")),
                  new FixtureBlock("network_cable", 14, 0, 0, arms("west=true")),
                  new FixtureBlock("fiber_patch_panel", 16, 0, 0, "facing=north"),
                  new FixtureBlock("network_cable", 16, 0, 1, arms("north=true"))),
              new double[] {14.5, 2.2, 4.2},
              new double[] {14.5, 0.6, 0.5}),
          // The Screen's casing next to the Terminal's: a 2x2 cluster beside a Terminal and a lone screen.
          new Fixture(
              "screen_closeup_vs_terminal",
              List.of(
                  new FixtureBlock("terminal_block", 19, 0, 0, "facing=south"),
                  new FixtureBlock("screen_block", 20, 0, 0, "facing=south"),
                  new FixtureBlock("screen_block", 21, 0, 0, "facing=south"),
                  new FixtureBlock("screen_block", 20, 1, 0, "facing=south"),
                  new FixtureBlock("screen_block", 21, 1, 0, "facing=south"),
                  new FixtureBlock("screen_block", 23, 0, 0, "facing=south")),
              new double[] {21.0, 1.9, 4.4},
              new double[] {21.0, 1.0, 0.5}));

  private RadioVisualLayout() {}

  public static String suite() {
    return System.getProperty(SUITE_PROPERTY, "tech");
  }

  /** True when client checks run and the selected suite is {@code name}. */
  public static boolean active(String name) {
    return Boolean.getBoolean("ecm.clientChecks") && suite().equals(name);
  }

  private static boolean radioClass(Class<?> type) {
    return type.getName().startsWith("com.example.evanscomputermod.radio.");
  }

  private static String guessGroup(String path) {
    if (path.contains("wire") || path.contains("antenna") || path.contains("mast")) return "antenna";
    if (path.contains("dish")) return "dish";
    if (path.contains("amp")) return "amplifier";
    if (path.contains("burner") || path.contains("generator") || path.contains("sdr")) return "power";
    if (path.contains("access") || path.contains("wifi")) return "wifi";
    return "misc";
  }

  public static Layout compute() {
    var specs = new ArrayList<Spec>();
    var skipped = new ArrayList<String>();
    var listed = new HashSet<String>();
    for (var s : SPECS) {
      listed.add(s.id());
      if (BuiltInRegistries.BLOCK.containsKey(EvansComputerMod.id(s.id()))) specs.add(s);
      else if (!skipped.contains(s.id())) skipped.add(s.id());
    }
    // Radio blocks the list does not know yet, in registry order.
    for (var block : BuiltInRegistries.BLOCK) {
      var key = BuiltInRegistries.BLOCK.getKey(block);
      if (!key.getNamespace().equals(EvansComputerMod.MODID) || listed.contains(key.getPath()))
        continue;
      if (!radioClass(block.getClass())) continue;
      specs.add(new Spec(guessGroup(key.getPath()), key.getPath(), "", Setup.NONE));
    }
    specs.sort(Comparator.comparingInt(s -> GROUPS.indexOf(s.group())));

    var blocks = new ArrayList<Placed>();
    for (var s : specs) {
      var id = EvansComputerMod.id(s.id());
      Block block = BuiltInRegistries.BLOCK.get(id);
      BlockState state = block.defaultBlockState();
      var expected = new LinkedHashMap<Property<?>, Comparable<?>>();
      // Fronts face the camera, which looks north from +Z.
      for (var facing :
          List.<Property<Direction>>of(
              BlockStateProperties.HORIZONTAL_FACING, BlockStateProperties.FACING)) {
        if (state.hasProperty(facing)) {
          state = state.setValue(facing, Direction.SOUTH);
          expected.put(facing, Direction.SOUTH);
        }
      }
      for (var pair : s.props().split(",")) {
        if (pair.isBlank()) continue;
        var kv = pair.split("=", 2);
        Property<?> property = block.getStateDefinition().getProperty(kv[0].trim());
        if (property == null)
          throw new IllegalStateException(s.id() + " has no property " + kv[0]);
        state = set(state, property, kv[1].trim(), expected);
      }
      blocks.add(
          new Placed(
              s.group(), id, state, expected, new BlockPos(blocks.size() * SPACING, Y, Z), s.setup()));
    }

    var items = new ArrayList<ResourceLocation>();
    for (var path : ITEM_SPECS) {
      var id = EvansComputerMod.id(path);
      if (BuiltInRegistries.ITEM.containsKey(id)) items.add(id);
      else skipped.add("item:" + path);
    }
    for (Item item : BuiltInRegistries.ITEM) {
      var id = BuiltInRegistries.ITEM.getKey(item);
      if (!id.getNamespace().equals(EvansComputerMod.MODID) || items.contains(id)) continue;
      boolean radioBlockItem =
          item instanceof BlockItem bi && blocks.stream().anyMatch(p -> p.state().is(bi.getBlock()));
      if (radioBlockItem || radioClass(item.getClass())) items.add(id);
    }

    var views = new ArrayList<View>();
    int rowEnd = blocks.isEmpty() ? 0 : blocks.get(blocks.size() - 1).pos().getX();
    if (!blocks.isEmpty()) views.add(frame(OVERVIEW, List.copyOf(blocks), 0, rowEnd));
    for (var group : GROUPS) {
      var members = blocks.stream().filter(p -> p.group().equals(group)).toList();
      if (members.isEmpty()) continue;
      views.add(
          frame(
              "radio_closeup_" + group,
              members,
              members.get(0).pos().getX(),
              members.get(members.size() - 1).pos().getX()));
    }
    int fx = Layout.fixtureX0(rowEnd);
    for (var fixture : FIXTURES) {
      var members = new ArrayList<Placed>();
      for (var fb : fixture.blocks()) {
        var id = EvansComputerMod.id(fb.id());
        if (!BuiltInRegistries.BLOCK.containsKey(id)) {
          skipped.add(fixture.name() + ":" + fb.id());
          continue;
        }
        Block block = BuiltInRegistries.BLOCK.get(id);
        BlockState state = block.defaultBlockState();
        var expected = new LinkedHashMap<Property<?>, Comparable<?>>();
        for (var pair : fb.props().split(",")) {
          if (pair.isBlank()) continue;
          var kv = pair.split("=", 2);
          Property<?> property = block.getStateDefinition().getProperty(kv[0].trim());
          if (property == null)
            throw new IllegalStateException(fb.id() + " has no property " + kv[0]);
          state = set(state, property, kv[1].trim(), expected);
        }
        members.add(
            new Placed(
                "fixture",
                id,
                state,
                expected,
                new BlockPos(fx + fb.dx(), Y + fb.dy(), Z + fb.dz()),
                Setup.NONE));
      }
      if (members.isEmpty()) continue;
      blocks.addAll(members);
      views.add(lookAt(fixture.name(), fx, fixture.eye(), fixture.target(), members));
    }

    int x0 = ITEM_X0;
    int columns = Math.min(ITEM_COLUMNS, Math.max(1, items.size()));
    int rows = Math.max(1, (items.size() + ITEM_COLUMNS - 1) / ITEM_COLUMNS);
    views.add(itemsView(x0, x0 + columns - 1, rows));
    return new Layout(List.copyOf(blocks), List.copyOf(items), List.copyOf(skipped), List.copyOf(views));
  }

  private static <T extends Comparable<T>> BlockState set(
      BlockState state, Property<T> property, String value, Map<Property<?>, Comparable<?>> expected) {
    T parsed =
        property
            .getValue(value)
            .orElseThrow(() -> new IllegalStateException(property.getName() + "=" + value));
    expected.put(property, parsed);
    return state.setValue(property, parsed);
  }

  /** Camera south of the row, looking north and slightly down, far enough to fit [x0, x1]. */
  private static View frame(String name, List<Placed> blocks, int x0, int x1) {
    double span = x1 - x0 + 1;
    // ~100 degree horizontal FOV at 16:9: half the span over tan(50) plus a margin.
    double distance = Math.max(2.4, span * 0.42 + 0.8);
    double eyeAbove = Math.max(0.9, distance * 0.35);
    double cx = (x0 + x1) / 2.0 + 0.5;
    var feet = new Vec3(cx, Y + 0.5 + eyeAbove - 1.62, Z + 0.5 + distance);
    float pitch = (float) Math.toDegrees(Math.atan2(eyeAbove, distance));
    return new View(name, feet, 180, pitch, List.copyOf(blocks));
  }

  /** A camera at {@code eye} looking at {@code target}, both relative to the fixtures' origin. */
  private static View lookAt(String name, int fx, double[] eye, double[] target, List<Placed> blocks) {
    double ex = fx + eye[0], ey = Y + eye[1], ez = Z + eye[2];
    double dx = fx + target[0] - ex, dy = Y + target[1] - ey, dz = Z + target[2] - ez;
    float yaw = (float) Math.toDegrees(Math.atan2(-dx, dz));
    float pitch = (float) -Math.toDegrees(Math.atan2(dy, Math.sqrt(dx * dx + dz * dz)));
    return new View(name, new Vec3(ex, ey - 1.62, ez), yaw, pitch, List.copyOf(blocks));
  }

  private static View itemsView(int x0, int x1, int rows) {
    double cx = (x0 + x1) / 2.0 + 0.5;
    double distance = Math.max(1.8, Math.max(x1 - x0 + 1, rows * 1.8) * 0.42 + 0.2);
    // Frames hang in rows from Y upward, flat against the wall at Z - 1.
    double eye = Y + rows / 2.0 + 0.2;
    var feet = new Vec3(cx, eye - 1.62, Z + 0.1 + distance);
    float pitch = (float) Math.toDegrees(Math.atan2(0.2, distance));
    return new View(ITEMS, feet, 180, pitch, List.of());
  }
}
//?}
