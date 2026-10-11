package com.example.evanscomputermod.radio.conductor;

//? if <=1.21.1 {
import net.minecraft.core.BlockPos;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.sounds.SoundEvents;
import net.minecraft.sounds.SoundSource;
import net.minecraft.tags.ItemTags;
import net.minecraft.util.RandomSource;
import net.minecraft.world.InteractionHand;
import net.minecraft.world.ItemInteractionResult;
import net.minecraft.world.entity.player.Player;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.item.Items;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.block.state.StateDefinition;
import net.minecraft.world.level.block.state.properties.BooleanProperty;
import net.minecraft.world.level.block.state.properties.IntegerProperty;
import net.minecraft.world.phys.BlockHitResult;

/**
 * A bare copper conductor that weathers like vanilla 1.21 copper: four
 * stages (unaffected, exposed, weathered, oxidized) advanced by random ticks
 * at vanilla's base chance. Honeycomb waxes it (stops weathering); an axe
 * scrapes the wax off, then one stage of oxide per use. Each stage raises
 * the RF loss (see {@code ConductorSpec.OXIDATION_FACTOR}).
 */
public class CopperConductorBlock extends ConductorBlock {
    public static final IntegerProperty OXIDATION = IntegerProperty.create("oxidation", 0, 3);
    public static final BooleanProperty WAXED = BooleanProperty.create("waxed");
    /** Vanilla WeatheringCopper's per-random-tick chance. */
    private static final float WEATHER_CHANCE = 0.05688889F;

    public CopperConductorBlock(Properties properties, float thicknessPx, boolean collision) {
        super(properties, Role.CONDUCTOR, thicknessPx, collision);
    }

    @Override
    protected BlockState defaultState(BlockState s) {
        return s.setValue(OXIDATION, 0).setValue(WAXED, false);
    }

    @Override
    protected void createBlockStateDefinition(StateDefinition.Builder<Block, BlockState> builder) {
        super.createBlockStateDefinition(builder);
        builder.add(OXIDATION, WAXED);
    }

    @Override
    protected boolean isRandomlyTicking(BlockState state) {
        return !state.getValue(WAXED) && state.getValue(OXIDATION) < 3;
    }

    @Override
    protected void randomTick(BlockState state, ServerLevel level, BlockPos pos, RandomSource random) {
        if (random.nextFloat() < WEATHER_CHANCE) weather(level, pos, state);
    }

    /** Advance one oxidation stage (no-op when waxed or fully oxidized). */
    public static void weather(Level level, BlockPos pos, BlockState state) {
        if (state.getValue(WAXED) || state.getValue(OXIDATION) >= 3) return;
        level.setBlock(pos, state.setValue(OXIDATION, state.getValue(OXIDATION) + 1), Block.UPDATE_ALL);
        com.example.evanscomputermod.radio.antenna.AntennaManager.blockChanged(level, pos);
    }

    @Override
    protected ItemInteractionResult useItemOn(ItemStack stack, BlockState state, Level level, BlockPos pos, Player player,
                                              InteractionHand hand, BlockHitResult hit) {
        if (stack.is(Items.HONEYCOMB) && !state.getValue(WAXED)) {
            if (!level.isClientSide()) {
                level.setBlock(pos, state.setValue(WAXED, true), Block.UPDATE_ALL);
                level.levelEvent(player, 3003, pos, 0);
                if (player == null || !player.getAbilities().instabuild) stack.shrink(1);
            }
            return ItemInteractionResult.sidedSuccess(level.isClientSide());
        }
        if (stack.is(ItemTags.AXES) && (state.getValue(WAXED) || state.getValue(OXIDATION) > 0)) {
            if (!level.isClientSide()) {
                boolean waxed = state.getValue(WAXED);
                BlockState now = waxed ? state.setValue(WAXED, false) : state.setValue(OXIDATION, state.getValue(OXIDATION) - 1);
                level.setBlock(pos, now, Block.UPDATE_ALL);
                level.playSound(null, pos, waxed ? SoundEvents.AXE_WAX_OFF : SoundEvents.AXE_SCRAPE, SoundSource.BLOCKS, 1, 1);
                level.levelEvent(player, waxed ? 3004 : 3005, pos, 0);
                if (player != null) stack.hurtAndBreak(1, player, net.minecraft.world.entity.LivingEntity.getSlotForHand(hand));
                com.example.evanscomputermod.radio.antenna.AntennaManager.blockChanged(level, pos);
            }
            return ItemInteractionResult.sidedSuccess(level.isClientSide());
        }
        return super.useItemOn(stack, state, level, pos, player, hand, hit);
    }
}
//?}
