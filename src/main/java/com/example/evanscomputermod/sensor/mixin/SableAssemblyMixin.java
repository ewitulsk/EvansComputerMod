package com.example.evanscomputermod.sensor.mixin;

//? if <=1.21.1 {
import com.example.evanscomputermod.sensor.wire.WireSableMover;
import dev.ryanhcode.sable.api.SubLevelAssemblyHelper;
import net.minecraft.core.BlockPos;
import net.minecraft.server.level.ServerLevel;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

/**
 * Sensor wires move with the blocks they are plugged into when Sable moves
 * blocks (only applied when Sable is installed, see SensorMixinPlugin).
 * Modelled on PowerGrid's SubLevelAssemblerMixin (Apache-2.0).
 */
@Mixin(value = SubLevelAssemblyHelper.class, remap = false)
public abstract class SableAssemblyMixin {
    @Inject(method = "moveBlocks", at = @At("HEAD"), require = 0)
    private static void evanscomputermod$moveSensorWires(ServerLevel level, SubLevelAssemblyHelper.AssemblyTransform transform,
                                                        Iterable<BlockPos> blocks, CallbackInfo ci) {
        WireSableMover.beforeMoveBlocks(level, transform, blocks);
    }
}
//?}
