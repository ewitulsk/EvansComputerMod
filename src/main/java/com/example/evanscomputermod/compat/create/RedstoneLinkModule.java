package com.example.evanscomputermod.compat.create;

//? if <=1.21.1 {
import com.example.evanscomputermod.api.module.IComputerModule;
import com.example.evanscomputermod.api.module.IModuleHost;
import net.minecraft.core.BlockPos;
import net.minecraft.nbt.CompoundTag;
import net.minecraft.server.level.ServerLevel;
import org.jetbrains.annotations.Nullable;

/**
 * The Redstone Link module: a {@link RedstoneLinkPeripheral} living in a
 * computer's bay. Its channel configuration is saved on the module item.
 */
public final class RedstoneLinkModule extends RedstoneLinkPeripheral implements IComputerModule {

    public RedstoneLinkModule(IModuleHost host, CompoundTag savedState) {
        super(new Location() {
            @Override
            @Nullable
            public ServerLevel level() {
                // Not gated on host.isAlive(): unload() still needs the level to
                // leave the link networks after the block entity is removed.
                return host.getLevel();
            }

            @Override
            public BlockPos pos() {
                return host.getPos();
            }

            @Override
            public void markDirty() {
                host.markDirty();
            }
        });
        load(savedState);
    }

    @Override
    public void onLoad() {
        load();
    }

    @Override
    public void onUnload() {
        unload();
    }

    @Override
    public void saveState(CompoundTag tag) {
        save(tag);
    }
}
//?}
