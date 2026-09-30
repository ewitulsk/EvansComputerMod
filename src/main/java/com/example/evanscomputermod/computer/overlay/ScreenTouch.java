package com.example.evanscomputermod.computer.overlay;

//? if <=1.21.1 {

import com.example.evanscomputermod.block.ScreenBlock;
import com.example.evanscomputermod.block.ScreenBlockEntity;
import com.example.evanscomputermod.block.TerminalBlockEntity;
import com.example.evanscomputermod.computer.TerminalDisplay;
import net.minecraft.core.BlockPos;
import net.minecraft.world.InteractionResult;
import net.minecraft.world.entity.player.Player;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.phys.BlockHitResult;

/**
 * Touch input on in-world Screens: right-clicking a Screen's face presses and
 * releases the left mouse button at that framebuffer pixel (sneaking: the
 * right button). Programs get it from {@code mouse_poll} like a click in the
 * terminal GUI, with the event's source byte set to "screen".
 */
public final class ScreenTouch {

    /** Mouse event source byte: the terminal GUI (0) or a Screen touch (1). */
    public static final byte SOURCE_SCREEN = 1;

    private ScreenTouch() {
    }

    public static InteractionResult use(BlockState state, Level level, BlockPos pos, Player player, BlockHitResult hit) {
        if (hit.getDirection() != state.getValue(ScreenBlock.FACING)) return InteractionResult.PASS;
        if (!(level.getBlockEntity(pos) instanceof ScreenBlockEntity sbe) || sbe.getClusterAnchor() == null
                || sbe.getOwnerTerminal() == null) {
            return InteractionResult.PASS;
        }
        if (level.isClientSide()) return sbe.isActive() ? InteractionResult.SUCCESS : InteractionResult.PASS;
        if (!(level.getBlockEntity(sbe.getClusterAnchor()) instanceof ScreenBlockEntity anchor)
                || !(level.getBlockEntity(sbe.getOwnerTerminal()) instanceof TerminalBlockEntity te)) {
            return InteractionResult.PASS;
        }
        TerminalDisplay d = te.getScreenDisplay();
        if (d == null) return InteractionResult.PASS;
        int[] px = ScreenGeometry.pixelAt(state.getValue(ScreenBlock.FACING), anchor.getBlockPos(),
                anchor.getClusterCols(), anchor.getClusterRows(), d.getGfxWidth(), d.getGfxHeight(), hit.getLocation());
        if (px == null) return InteractionResult.PASS;
        return te.onScreenTouch(px[0], px[1], player.isSecondaryUseActive() ? 1 : 0)
                ? InteractionResult.CONSUME : InteractionResult.PASS;
    }
}
//?}
