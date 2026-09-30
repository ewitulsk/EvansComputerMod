package com.example.evanscomputermod.computer.overlay.client;

//? if <=1.21.1 {

import com.example.evanscomputermod.block.ScreenBlock;
import com.example.evanscomputermod.block.ScreenBlockEntity;
import com.example.evanscomputermod.computer.TerminalDisplay;
import com.example.evanscomputermod.computer.overlay.ItemOverlays;
import com.example.evanscomputermod.computer.overlay.ScreenGeometry;
import net.minecraft.client.Minecraft;
import net.minecraft.core.BlockPos;
import net.minecraft.network.chat.Component;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.phys.BlockHitResult;
import net.minecraft.world.phys.HitResult;

/**
 * The in-world stand-in for a tooltip: looking at an item on a Screen shows
 * its name (and label, e.g. the stored count) above the hotbar.
 */
final class OverlayHover {

    private static ItemOverlays.Entry last;

    private OverlayHover() {
    }

    static void tick() {
        Minecraft mc = Minecraft.getInstance();
        if (mc.level == null || mc.player == null || !(mc.hitResult instanceof BlockHitResult hit)
                || hit.getType() != HitResult.Type.BLOCK) {
            last = null;
            return;
        }
        BlockState state = mc.level.getBlockState(hit.getBlockPos());
        if (!(state.getBlock() instanceof ScreenBlock)
                || !(mc.level.getBlockEntity(hit.getBlockPos()) instanceof ScreenBlockEntity sbe)
                || sbe.getClusterAnchor() == null
                || !(mc.level.getBlockEntity(sbe.getClusterAnchor()) instanceof ScreenBlockEntity anchor)) {
            last = null;
            return;
        }
        if (hit.getDirection() != state.getValue(ScreenBlock.FACING)) {
            last = null;
            return;
        }
        BlockPos owner = anchor.getOwnerTerminal();
        ClientItemOverlays.Overlay o = ClientItemOverlays.get(owner, ItemOverlays.TARGET_SCREEN);
        TerminalDisplay d = anchor.clientDisplay;
        if (o == null || d == null) {
            last = null;
            return;
        }
        int[] px = ScreenGeometry.pixelAt(state.getValue(ScreenBlock.FACING), anchor.getBlockPos(),
                anchor.getClusterCols(), anchor.getClusterRows(), d.getGfxWidth(), d.getGfxHeight(), hit.getLocation());
        ItemOverlays.Entry found = null;
        if (px != null) {
            for (ItemOverlays.Entry e : o.entries) {
                if ((e.flags() & ItemOverlays.FLAG_DIMMED) == 0 && px[0] >= e.x() && px[0] < e.x() + e.size()
                        && px[1] >= e.y() && px[1] < e.y() + e.size()) {
                    found = e;
                }
            }
        }
        if (found != null && !found.equals(last)) {
            ItemStack stack = o.stack(found);
            if (stack != null) {
                Component name = stack.getHoverName();
                mc.gui.setOverlayMessage(found.label().isEmpty() ? name
                        : Component.empty().append(name).append(Component.literal("  x" + found.label())), false);
            }
        }
        last = found;
    }
}
//?}
