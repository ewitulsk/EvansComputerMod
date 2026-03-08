package com.example.customworld.command;

import com.example.customworld.CustomWorldMod;
import com.mojang.brigadier.CommandDispatcher;
import com.mojang.brigadier.context.CommandContext;
import net.minecraft.commands.CommandSourceStack;
import net.minecraft.commands.Commands;
import net.minecraft.core.BlockPos;
import net.minecraft.core.registries.Registries;
import net.minecraft.network.chat.Component;
import net.minecraft.resources.ResourceKey;
import net.minecraft.resources.ResourceLocation;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.server.level.ServerPlayer;
import net.minecraft.world.level.Level;

/**
 * Command to teleport players to our custom dimension.
 * Usage: /tpcustomworld
 * 
 * This is a simple teleport command for testing. A proper implementation
 * would use portal mechanics or other in-game methods.
 */
public class TeleportDimensionCommand {

    public static final ResourceKey<Level> CUSTOM_DIMENSION = ResourceKey.create(
            Registries.DIMENSION,
            new ResourceLocation(CustomWorldMod.MODID, "custom_dimension")
    );

    public static void register(CommandDispatcher<CommandSourceStack> dispatcher) {
        dispatcher.register(
                Commands.literal("tpcustomworld")
                        .requires(source -> source.hasPermission(2)) // Requires op level 2
                        .executes(TeleportDimensionCommand::teleportToCustomWorld)
        );

        dispatcher.register(
                Commands.literal("tpoverworld")
                        .requires(source -> source.hasPermission(2))
                        .executes(TeleportDimensionCommand::teleportToOverworld)
        );

        CustomWorldMod.LOGGER.info("Registered dimension teleport commands");
    }

    private static int teleportToCustomWorld(CommandContext<CommandSourceStack> context) {
        CommandSourceStack source = context.getSource();

        if (source.getEntity() instanceof ServerPlayer player) {
            ServerLevel customWorld = source.getServer().getLevel(CUSTOM_DIMENSION);

            if (customWorld == null) {
                source.sendFailure(Component.literal("Custom dimension not found! Make sure the dimension is properly configured."));
                return 0;
            }

            // Find a safe spawn position
            BlockPos spawnPos = findSafeSpawnPosition(customWorld, player.blockPosition());

            // Teleport the player
            player.teleportTo(
                    customWorld,
                    spawnPos.getX() + 0.5,
                    spawnPos.getY(),
                    spawnPos.getZ() + 0.5,
                    player.getYRot(),
                    player.getXRot()
            );

            source.sendSuccess(() -> Component.literal("Teleported to Custom World at " + 
                    spawnPos.getX() + ", " + spawnPos.getY() + ", " + spawnPos.getZ()), true);
            return 1;
        }

        source.sendFailure(Component.literal("This command can only be used by players!"));
        return 0;
    }

    private static int teleportToOverworld(CommandContext<CommandSourceStack> context) {
        CommandSourceStack source = context.getSource();

        if (source.getEntity() instanceof ServerPlayer player) {
            ServerLevel overworld = source.getServer().getLevel(Level.OVERWORLD);

            if (overworld == null) {
                source.sendFailure(Component.literal("Overworld not found!"));
                return 0;
            }

            // Teleport to spawn or current XZ position
            BlockPos spawnPos = overworld.getSharedSpawnPos();

            player.teleportTo(
                    overworld,
                    spawnPos.getX() + 0.5,
                    spawnPos.getY(),
                    spawnPos.getZ() + 0.5,
                    player.getYRot(),
                    player.getXRot()
            );

            source.sendSuccess(() -> Component.literal("Teleported back to Overworld!"), true);
            return 1;
        }

        source.sendFailure(Component.literal("This command can only be used by players!"));
        return 0;
    }

    /**
     * Find a safe position to spawn the player.
     * In our custom dimension, we want to spawn them on top of a pillar.
     */
    private static BlockPos findSafeSpawnPosition(ServerLevel level, BlockPos currentPos) {
        // Spawn at a pillar location (pillars are at multiples of 8)
        int x = (currentPos.getX() / 8) * 8;
        int z = (currentPos.getZ() / 8) * 8;
        
        // Start from a high position and find a safe spot
        int minY = level.getMinBuildHeight();
        int pillarHeight = 64 + ((x + z) % 32);
        int y = minY + pillarHeight + 2; // Spawn on top of pillar

        return new BlockPos(x, y, z);
    }
}
