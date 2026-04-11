package com.example.evanscomputermod.block;

import net.minecraft.core.BlockPos;
import net.minecraft.world.level.block.state.BlockState;

/**
 * Switch block entity — identical to {@link TerminalBlockEntity} in every
 * respect except it boots the `switch_os` WASM kernel by default instead of
 * `terminal_os`. All networking, redstone, framebuffer, and computer logic is
 * inherited unchanged, and `instanceof TerminalBlockEntity` checks elsewhere
 * (cables, interface blocks, menus) automatically cover switches too.
 */
public class SwitchBlockEntity extends TerminalBlockEntity {

    public SwitchBlockEntity(BlockPos pos, BlockState state) {
        super(ModBlockEntities.SWITCH_BLOCK_ENTITY.get(), pos, state);
    }

    @Override
    protected String defaultWasmModule() {
        return "switch_os";
    }
}
