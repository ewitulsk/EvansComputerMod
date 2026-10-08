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

  /** Items shown in frames besides the block items of the row. */
  static final List<String> ITEM_SPECS = List.of("handheld_radio", "controller_receiver_module");

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
    if (!blocks.isEmpty()) views.add(frame(OVERVIEW, blocks, 0, blocks.get(blocks.size() - 1).pos().getX()));
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
    double distance = Math.max(3.2, span * 0.55 + 1.5);
    double eyeAbove = Math.max(1.2, distance * 0.3);
    double cx = (x0 + x1) / 2.0 + 0.5;
    var feet = new Vec3(cx, Y + 0.5 + eyeAbove - 1.62, Z + 0.5 + distance);
    float pitch = (float) Math.toDegrees(Math.atan2(eyeAbove, distance));
    return new View(name, feet, 180, pitch, List.copyOf(blocks));
  }

  private static View itemsView(int x0, int x1, int rows) {
    double cx = (x0 + x1) / 2.0 + 0.5;
    double distance = Math.max(3.0, Math.max(x1 - x0 + 1, rows * 1.8) * 0.55 + 1.0);
    // Frames hang in rows from Y upward, flat against the wall at Z - 1.
    double eye = Y + rows / 2.0 + 0.2;
    var feet = new Vec3(cx, eye - 1.62, Z + 0.1 + distance);
    float pitch = (float) Math.toDegrees(Math.atan2(0.2, distance));
    return new View(ITEMS, feet, 180, pitch, List.of());
  }
}
//?}
