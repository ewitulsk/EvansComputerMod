package com.example.evanscomputermod.worldgen;

//? if <=1.21.1 {
import com.example.evanscomputermod.block.*;
import com.example.evanscomputermod.computer.WorldNetwork;
import java.util.*;
import net.minecraft.core.*;
import net.minecraft.core.registries.Registries;
import net.minecraft.resources.ResourceLocation;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.world.level.levelgen.Heightmap;
import net.minecraft.world.level.levelgen.structure.StructureStart;

/** Resolves the generated structure, not an assumed unrotated template offset. */
public final class TechVillageLocator {
  public record Visit(BlockPos arrival, float yaw, StructureStart start, Set<UUID> computers) {}

  public static Visit visit(ServerLevel level, int number) {
    var data = WorldNetwork.get(level);
    data.plan(level);
    var site = data.villages.get(number - 1);
    var structure =
        level
            .registryAccess()
            .registryOrThrow(Registries.STRUCTURE)
            .get(ResourceLocation.fromNamespaceAndPath("evanscomputermod", "tech_village"));
    if (structure == null)
      throw new IllegalStateException("Tech Village structure is absent from the loaded data pack");
    var chunk = level.getChunk(site.x() >> 4, site.z() >> 4);
    var start = chunk.getStartForStructure(structure);
    if (start == null || !start.isValid())
      throw new IllegalStateException(
          "Village "
              + number
              + " did not generate at "
              + site.x()
              + ", "
              + site.z()
              + ". Check world structure generation and the log; teleport cancelled.");
    var box = start.getBoundingBox();
    Set<UUID> ids = new HashSet<>();
    BlockPos entrance = null;
    float yaw = 0;
    UUID router = data.identity(level, number, "isp.router");
    for (int x = box.minX() >> 4; x <= box.maxX() >> 4; x++)
      for (int z = box.minZ() >> 4; z <= box.maxZ() >> 4; z++)
        for (var be : level.getChunk(x, z).getBlockEntities().values())
          if (be instanceof TerminalBlockEntity terminal && box.isInside(be.getBlockPos())) {
            ids.add(terminal.getComputerId());
            if (router.equals(terminal.getComputerId())) {
              var front = terminal.getBlockState().getValue(TerminalBlock.FACING);
              // Authored ISP door is three blocks right of the router and
              // six blocks forward. Arrive four blocks outside the door.
              entrance =
                  terminal.getBlockPos().relative(front, 10).relative(front.getClockWise(), 3);
              yaw = front.getOpposite().toYRot();
            }
          }
    if (entrance == null) {
      var first = start.getPieces().get(0).getBoundingBox();
      entrance = new BlockPos((first.minX() + first.maxX()) / 2, 0, first.minZ() - 2);
    }
    int y =
        level.getHeight(
            Heightmap.Types.MOTION_BLOCKING_NO_LEAVES, entrance.getX(), entrance.getZ());
    return new Visit(
        new BlockPos(entrance.getX(), y, entrance.getZ()), yaw, start, Set.copyOf(ids));
  }
}
//?}
