package com.example.evanscomputermod.worldgen;

//? if <=1.21.1 {
import com.example.evanscomputermod.block.*;
import com.example.evanscomputermod.computer.WorldNetwork;
import java.util.*;
import net.minecraft.core.*;
import net.minecraft.core.registries.Registries;
import net.minecraft.resources.ResourceLocation;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.world.level.ChunkPos;
import net.minecraft.world.level.block.Mirror;
import net.minecraft.world.level.levelgen.structure.*;
import net.minecraft.world.level.levelgen.structure.templatesystem.StructureTemplate;

/** Resolves the generated structure, not an assumed unrotated template offset. */
public final class TechVillageLocator {
  /** Outside the ISP's front door, in the ISP template's coordinates (door at z = 15). */
  public static final BlockPos ARRIVAL_LOCAL = new BlockPos(10, 1, 19);

  public record Visit(
      BlockPos arrival,
      float yaw,
      StructureStart start,
      PoolElementStructurePiece isp,
      PoolElementStructurePiece datacenter,
      TechNetworkPiece network,
      Set<UUID> computers) {}

  /** Template location of a placed pool piece, or null. */
  public static ResourceLocation location(StructurePiece piece) {
    if (piece instanceof PoolElementStructurePiece pe
        && pe.getElement() instanceof net.minecraft.world.level.levelgen.structure.pools.SinglePoolElement single)
      return ((com.example.evanscomputermod.worldgen.mixin.SinglePoolElementAccessor) single)
          .ecm$template()
          .left()
          .orElse(null);
    return null;
  }

  /**
   * World position of the terminal with provisioning role {@code role} ({@code isp.router},
   * {@code datacenter.web}, {@code datacenter.chat}) in the ISP or the Data Center, or null.
   */
  public static BlockPos role(ServerLevel level, Visit visit, String role) {
    for (var piece : new PoolElementStructurePiece[] {visit.isp(), visit.datacenter()}) {
      if (piece == null) continue;
      var scan = TemplateScan.of(level.getStructureManager(), location(piece));
      if (scan.isEmpty()) continue;
      BlockPos local = scan.get().findNbt("ecmRole", role);
      if (local != null) return world(piece, local);
    }
    return null;
  }

  private static final net.minecraft.server.level.TicketType<ChunkPos> HOLD =
      net.minecraft.server.level.TicketType.create(
          "ecm_village_hold", java.util.Comparator.comparingLong(ChunkPos::toLong));

  /** Keeps (or releases) every chunk of a village loaded, e.g. while tests use its computers. */
  public static void hold(ServerLevel level, StructureStart start, boolean hold) {
    var box = start.getBoundingBox();
    for (int x = box.minX() >> 4; x <= box.maxX() >> 4; x++)
      for (int z = box.minZ() >> 4; z <= box.maxZ() >> 4; z++) {
        var c = new ChunkPos(x, z);
        if (hold) level.getChunkSource().addRegionTicket(HOLD, c, 1, c);
        else level.getChunkSource().removeRegionTicket(HOLD, c, 1, c);
      }
  }

  public static Structure structure(ServerLevel level, String style) {
    var structure =
        level
            .registryAccess()
            .registryOrThrow(Registries.STRUCTURE)
            .get(ResourceLocation.fromNamespaceAndPath("evanscomputermod", "tech_village_" + style));
    if (structure == null)
      throw new IllegalStateException(
          "Tech Village structure tech_village_" + style + " is absent from the loaded data pack");
    return structure;
  }

  /** Template position to world position for a placed pool piece. */
  public static BlockPos world(PoolElementStructurePiece piece, BlockPos local) {
    return piece
        .getPosition()
        .offset(StructureTemplate.transform(local, Mirror.NONE, piece.getRotation(), BlockPos.ZERO));
  }

  public static Visit visit(ServerLevel level, int number) {
    var data = WorldNetwork.get(level);
    data.plan(level);
    var site = data.villages.get(number - 1);
    var chunk = level.getChunk(site.x() >> 4, site.z() >> 4);
    var start = chunk.getStartForStructure(structure(level, site.style()));
    if (start == null || !start.isValid())
      throw new IllegalStateException(
          "Village "
              + number
              + " ("
              + site.style()
              + ") did not generate at "
              + site.x()
              + ", "
              + site.z()
              + ". Tech Villages generate in fresh normal and superflat worlds with structures"
              + " enabled; chunks generated before this version have no village. Teleport cancelled.");
    return resolve(level, number, start);
  }

  /** Reads the placed village (loading its chunks): ISP piece, network piece, computers, arrival. */
  public static Visit resolve(ServerLevel level, int number, StructureStart start) {
    var data = WorldNetwork.get(level);
    PoolElementStructurePiece isp = (PoolElementStructurePiece) start.getPieces().get(0);
    PoolElementStructurePiece datacenter =
        start.getPieces().stream()
            .filter(p -> location(p) != null && location(p).getPath().startsWith("tech_village/datacenter_"))
            .map(p -> (PoolElementStructurePiece) p)
            .findFirst()
            .orElse(null);
    TechNetworkPiece network =
        start.getPieces().stream()
            .filter(p -> p instanceof TechNetworkPiece)
            .map(p -> (TechNetworkPiece) p)
            .findFirst()
            .orElse(null);
    var box = start.getBoundingBox();
    Set<UUID> ids = new HashSet<>();
    for (int x = box.minX() >> 4; x <= box.maxX() >> 4; x++)
      for (int z = box.minZ() >> 4; z <= box.maxZ() >> 4; z++)
        for (var be : level.getChunk(x, z).getBlockEntities().values())
          if (be instanceof TerminalBlockEntity terminal && box.isInside(be.getBlockPos()))
            ids.add(terminal.getComputerId());
    BlockPos arrival = world(isp, ARRIVAL_LOCAL);
    float yaw = isp.getRotation().rotate(Direction.NORTH).toYRot();
    int y =
        level.getHeight(
            net.minecraft.world.level.levelgen.Heightmap.Types.MOTION_BLOCKING_NO_LEAVES,
            arrival.getX(),
            arrival.getZ());
    return new Visit(
        new BlockPos(arrival.getX(), y, arrival.getZ()), yaw, start, isp, datacenter, network, Set.copyOf(ids));
  }

  /**
   * Generates a village's structure directly at its planned site, the way {@code /place
   * structure} does. Only for GameTests, whose world is created with structures disabled.
   */
  public static StructureStart placeForTest(ServerLevel level, int number) {
    var data = WorldNetwork.get(level);
    data.plan(level);
    var site = data.villages.get(number - 1);
    var generator = level.getChunkSource().getGenerator();
    var structure = structure(level, site.style());
    var start =
        structure.generate(
            level.registryAccess(),
            generator,
            generator.getBiomeSource(),
            level.getChunkSource().randomState(),
            level.getStructureManager(),
            level.getSeed(),
            new ChunkPos(site.x() >> 4, site.z() >> 4),
            0,
            level,
            b -> true);
    if (!start.isValid()) throw new IllegalStateException("Village " + number + " did not generate");
    var box = start.getBoundingBox();
    ChunkPos.rangeClosed(
            new ChunkPos(SectionPos.blockToSectionCoord(box.minX()), SectionPos.blockToSectionCoord(box.minZ())),
            new ChunkPos(SectionPos.blockToSectionCoord(box.maxX()), SectionPos.blockToSectionCoord(box.maxZ())))
        .forEach(
            c ->
                start.placeInChunk(
                    level,
                    level.structureManager(),
                    generator,
                    level.getRandom(),
                    new net.minecraft.world.level.levelgen.structure.BoundingBox(
                        c.getMinBlockX(),
                        level.getMinBuildHeight(),
                        c.getMinBlockZ(),
                        c.getMaxBlockX(),
                        level.getMaxBuildHeight(),
                        c.getMaxBlockZ()),
                    c));
    // /place-style placement snaps terrain-matching streets to the live surface, which
    // counts the overhead fiber as ground; natural generation places the village first.
    var ring = data.ring(level);
    ChunkPos.rangeClosed(
            new ChunkPos(SectionPos.blockToSectionCoord(box.minX()), SectionPos.blockToSectionCoord(box.minZ())),
            new ChunkPos(SectionPos.blockToSectionCoord(box.maxX()), SectionPos.blockToSectionCoord(box.maxZ())))
        .forEach(c -> FiberLineFeature.reassert(level, ring, c.x, c.z));
    return start;
  }
}
//?}
