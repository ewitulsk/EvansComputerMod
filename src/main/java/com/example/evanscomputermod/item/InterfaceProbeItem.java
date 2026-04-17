package com.example.evanscomputermod.item;

import com.example.evanscomputermod.block.InterfaceBlock;
import com.example.evanscomputermod.block.TerminalBlock;
import com.example.evanscomputermod.block.TerminalBlockEntity;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.network.chat.Component;
import net.minecraft.world.InteractionResult;
import net.minecraft.world.entity.player.Player;
import net.minecraft.world.item.Item;
import net.minecraft.world.item.context.UseOnContext;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.block.Block;
import org.jspecify.annotations.Nullable;

import java.util.ArrayDeque;
import java.util.HashSet;
import java.util.Queue;
import java.util.Set;

/**
 * Right-click a computer or interface block face to print the name
 * (eth0, eth1, ...) of the network interface exposed on that face.
 */
public class InterfaceProbeItem extends Item {

    // Safety bound on the backward BFS from an InterfaceBlock to its owning
    // computer — stops a pathological interface-block mesh from stalling the
    // server thread.
    private static final int BFS_POSITION_CAP = 256;

    public InterfaceProbeItem(Properties properties) {
        super(properties);
    }

    @Override
    public InteractionResult useOn(UseOnContext ctx) {
        Level level = ctx.getLevel();
        if (level.isClientSide()) return InteractionResult.SUCCESS;

        Player player = ctx.getPlayer();
        if (player == null) return InteractionResult.PASS;

        BlockPos clickedPos = ctx.getClickedPos();
        Direction clickedFace = ctx.getClickedFace();
        Block clickedBlock = level.getBlockState(clickedPos).getBlock();

        boolean isTerminal = clickedBlock instanceof TerminalBlock;
        boolean isInterface = clickedBlock instanceof InterfaceBlock;
        if (!isTerminal && !isInterface) return InteractionResult.PASS;

        TerminalBlockEntity owner = isTerminal
                ? (level.getBlockEntity(clickedPos) instanceof TerminalBlockEntity t ? t : null)
                : findOwningTerminal(level, clickedPos);

        if (owner == null) {
            player.sendSystemMessage(Component.literal(
                    "§cInterface Probe: this interface block isn't connected to a computer."));
            return InteractionResult.SUCCESS;
        }

        BlockPos exitPos = clickedPos.relative(clickedFace);
        int index = owner.findInterfaceIndexByExitPos(exitPos);

        BlockPos cpos = owner.getBlockPos();
        String locatorTag = String.format("§7(computer at %d, %d, %d)", cpos.getX(), cpos.getY(), cpos.getZ());

        if (index < 0) {
            player.sendSystemMessage(Component.literal(
                    "§eInterface Probe: no interface on that face " + locatorTag));
        } else {
            player.sendSystemMessage(Component.literal(
                    "§aInterface Probe: §feth" + index + " §a" + locatorTag));
        }
        return InteractionResult.SUCCESS;
    }

    /**
     * Walk outward through connected {@link InterfaceBlock}s until an adjacent
     * {@link TerminalBlock} is found, and return its BlockEntity. Returns null
     * if no owning terminal is reachable within {@link #BFS_POSITION_CAP} steps.
     */
    private static @Nullable TerminalBlockEntity findOwningTerminal(Level level, BlockPos startInterface) {
        Set<BlockPos> visited = new HashSet<>();
        Queue<BlockPos> queue = new ArrayDeque<>();
        visited.add(startInterface);
        queue.add(startInterface);

        while (!queue.isEmpty() && visited.size() < BFS_POSITION_CAP) {
            BlockPos pos = queue.poll();
            for (Direction dir : Direction.values()) {
                BlockPos n = pos.relative(dir);
                if (!visited.add(n)) continue;
                Block b = level.getBlockState(n).getBlock();
                if (b instanceof TerminalBlock) {
                    if (level.getBlockEntity(n) instanceof TerminalBlockEntity t) return t;
                } else if (b instanceof InterfaceBlock) {
                    queue.add(n);
                }
            }
        }
        return null;
    }
}
