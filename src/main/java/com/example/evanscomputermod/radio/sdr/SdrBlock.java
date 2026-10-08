package com.example.evanscomputermod.radio.sdr;

//? if <=1.21.1 {
import com.mojang.serialization.MapCodec;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.network.chat.Component;
import net.minecraft.world.InteractionResult;
import net.minecraft.world.entity.player.Player;
import net.minecraft.world.item.context.BlockPlaceContext;
import net.minecraft.world.level.BlockGetter;
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
import net.minecraft.world.level.block.state.properties.DirectionProperty;
import net.minecraft.world.phys.BlockHitResult;
import net.minecraft.world.phys.shapes.CollisionContext;
import net.minecraft.world.phys.shapes.Shapes;
import net.minecraft.world.phys.shapes.VoxelShape;
import org.jetbrains.annotations.Nullable;

/**
 * SDR block: a software-defined radio peripheral that receives and transmits
 * raw IQ through {@code /dev/sdr}. Three tiers differ in tuning range, sample
 * rate, ADC depth and transmit (see {@link SdrTier}).
 */
public class SdrBlock extends BaseEntityBlock {
    public static final DirectionProperty FACING = BlockStateProperties.HORIZONTAL_FACING;
    private static final VoxelShape SHAPE = Shapes.or(Block.box(2, 0, 3, 14, 6, 13), Block.box(11, 6, 10, 12, 16, 11));

    private final SdrTier tier;

    public SdrBlock(SdrTier tier, Properties properties) {
        super(properties);
        this.tier = tier;
        registerDefaultState(stateDefinition.any().setValue(FACING, Direction.NORTH));
    }

    public SdrTier tier() {
        return tier;
    }

    @Override
    protected MapCodec<? extends BaseEntityBlock> codec() {
        return simpleCodec(p -> new SdrBlock(tier, p));
    }

    @Override
    protected void createBlockStateDefinition(StateDefinition.Builder<Block, BlockState> builder) {
        builder.add(FACING);
    }

    @Override
    public BlockState getStateForPlacement(BlockPlaceContext ctx) {
        return defaultBlockState().setValue(FACING, ctx.getHorizontalDirection().getOpposite());
    }

    @Override
    protected VoxelShape getShape(BlockState state, BlockGetter level, BlockPos pos, CollisionContext ctx) {
        return SHAPE;
    }

    @Override
    protected RenderShape getRenderShape(BlockState state) {
        return RenderShape.MODEL;
    }

    @Override
    public @Nullable BlockEntity newBlockEntity(BlockPos pos, BlockState state) {
        return new SdrBlockEntity(pos, state);
    }

    @Override
    public @Nullable <T extends BlockEntity> BlockEntityTicker<T> getTicker(Level level, BlockState state, BlockEntityType<T> type) {
        return level.isClientSide() ? null : createTickerHelper(type, RadioSdrContent.SDR_BE.get(), SdrBlockEntity::serverTick);
    }

    @Override
    protected InteractionResult useWithoutItem(BlockState state, Level level, BlockPos pos, Player player, BlockHitResult hit) {
        if (!level.isClientSide() && level.getBlockEntity(pos) instanceof SdrBlockEntity be) {
            var r = be.getPeripheral().radio();
            player.displayClientMessage(Component.translatable("message.evanscomputermod.sdr.status",
                    Component.translatable("block.evanscomputermod." + tier.id()),
                    String.format("%.4f", r.centerHz() / 1e6), r.sampleRate(), String.format("%.0f", r.gainDb())), true);
        }
        return InteractionResult.sidedSuccess(level.isClientSide());
    }
}
//?}
