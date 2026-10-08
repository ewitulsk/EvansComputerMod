package com.example.evanscomputermod.radio.power;

//? if <=1.21.1 {
import com.mojang.serialization.MapCodec;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.core.particles.ParticleTypes;
import net.minecraft.sounds.SoundEvents;
import net.minecraft.sounds.SoundSource;
import net.minecraft.util.RandomSource;
import net.minecraft.world.InteractionHand;
import net.minecraft.world.InteractionResult;
import net.minecraft.world.ItemInteractionResult;
import net.minecraft.world.entity.player.Player;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.item.context.BlockPlaceContext;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.block.BaseEntityBlock;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.RenderShape;
import net.minecraft.world.level.block.entity.BlockEntity;
import net.minecraft.world.level.block.entity.BlockEntityTicker;
import net.minecraft.world.level.block.entity.BlockEntityType;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.block.state.StateDefinition;
import net.minecraft.world.level.block.state.properties.BlockStateProperties;
import net.minecraft.world.level.block.state.properties.BooleanProperty;
import net.minecraft.world.level.block.state.properties.DirectionProperty;
import net.minecraft.world.phys.BlockHitResult;
import org.jetbrains.annotations.Nullable;

/**
 * Burner Generator: burns any furnace fuel (the {@code neoforge:furnace_fuels}
 * data map, so modded fuels and pack changes apply) for a weak, steady FE
 * output — about furnace efficiency, enough for a 100 W amplifier. Pushes FE to
 * every neighbour that accepts it.
 */
public class BurnerGeneratorBlock extends BaseEntityBlock {
    public static final MapCodec<BurnerGeneratorBlock> CODEC = simpleCodec(BurnerGeneratorBlock::new);
    public static final DirectionProperty FACING = BlockStateProperties.HORIZONTAL_FACING;
    public static final BooleanProperty LIT = BlockStateProperties.LIT;

    public BurnerGeneratorBlock(Properties properties) {
        super(properties);
        registerDefaultState(stateDefinition.any().setValue(FACING, Direction.NORTH).setValue(LIT, false));
    }

    @Override
    protected MapCodec<? extends BaseEntityBlock> codec() {
        return CODEC;
    }

    @Override
    protected void createBlockStateDefinition(StateDefinition.Builder<Block, BlockState> builder) {
        builder.add(FACING, LIT);
    }

    @Override
    public BlockState getStateForPlacement(BlockPlaceContext ctx) {
        return defaultBlockState().setValue(FACING, ctx.getHorizontalDirection().getOpposite());
    }

    @Override
    protected RenderShape getRenderShape(BlockState state) {
        return RenderShape.MODEL;
    }

    @Override
    public @Nullable BlockEntity newBlockEntity(BlockPos pos, BlockState state) {
        return new BurnerGeneratorBlockEntity(pos, state);
    }

    @Override
    public @Nullable <T extends BlockEntity> BlockEntityTicker<T> getTicker(Level level, BlockState state, BlockEntityType<T> type) {
        return level.isClientSide() ? null
                : createTickerHelper(type, RadioPowerContent.BURNER_GENERATOR_BE.get(), BurnerGeneratorBlockEntity::serverTick);
    }

    /** Right-click with fuel: put it in. */
    @Override
    protected ItemInteractionResult useItemOn(ItemStack stack, BlockState state, Level level, BlockPos pos, Player player,
                                              InteractionHand hand, BlockHitResult hit) {
        if (!(level.getBlockEntity(pos) instanceof BurnerGeneratorBlockEntity be) || !BurnerGeneratorBlockEntity.isFuel(stack))
            return ItemInteractionResult.PASS_TO_DEFAULT_BLOCK_INTERACTION;
        if (!level.isClientSide()) {
            ItemStack rest = be.fuel().insertItem(0, stack.copy(), false);
            if (!player.getAbilities().instabuild) player.setItemInHand(hand, rest);
        }
        return ItemInteractionResult.sidedSuccess(level.isClientSide());
    }

    /** Empty-hand right-click: status (energy, burn time, disabled). */
    @Override
    protected InteractionResult useWithoutItem(BlockState state, Level level, BlockPos pos, Player player, BlockHitResult hit) {
        if (!level.isClientSide() && level.getBlockEntity(pos) instanceof BurnerGeneratorBlockEntity be) {
            player.displayClientMessage(be.status(), true);
        }
        return InteractionResult.sidedSuccess(level.isClientSide());
    }

    @Override
    protected void onRemove(BlockState state, Level level, BlockPos pos, BlockState newState, boolean moved) {
        if (!state.is(newState.getBlock()) && level.getBlockEntity(pos) instanceof BurnerGeneratorBlockEntity be) {
            Block.popResource(level, pos, be.fuel().getStackInSlot(0));
        }
        super.onRemove(state, level, pos, newState, moved);
    }

    @Override
    public void animateTick(BlockState state, Level level, BlockPos pos, RandomSource random) {
        if (!state.getValue(LIT)) return;
        double x = pos.getX() + 0.5, y = pos.getY() + 0.1, z = pos.getZ() + 0.5;
        if (random.nextDouble() < 0.1)
            level.playLocalSound(x, y, z, SoundEvents.FURNACE_FIRE_CRACKLE, SoundSource.BLOCKS, 1, 1, false);
        Direction d = state.getValue(FACING);
        double ox = d.getStepX() * 0.52, oz = d.getStepZ() * 0.52;
        level.addParticle(ParticleTypes.SMOKE, x + ox, y + 0.3 + random.nextDouble() * 0.2, z + oz, 0, 0, 0);
        level.addParticle(ParticleTypes.FLAME, x + ox, y + 0.2 + random.nextDouble() * 0.1, z + oz, 0, 0, 0);
    }
}
//?}
