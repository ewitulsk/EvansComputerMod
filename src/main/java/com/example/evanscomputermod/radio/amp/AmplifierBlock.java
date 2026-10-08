package com.example.evanscomputermod.radio.amp;

//? if <=1.21.1 {
import com.mojang.serialization.MapCodec;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.network.chat.Component;
import net.minecraft.world.InteractionResult;
import net.minecraft.world.entity.LivingEntity;
import net.minecraft.world.entity.player.Player;
import net.minecraft.world.item.ItemStack;
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
import net.minecraft.world.phys.shapes.VoxelShape;
import org.jetbrains.annotations.Nullable;

import java.util.List;

/**
 * RF power amplifier (100 W / 1 kW / 10 kW). Sits in the coax between an
 * exciter (an SDR, touching it or through coax) and the antenna; takes FE on
 * any side. Its two coax ports have no direction. Right-click for status:
 * output, SWR, reflected power, temperature, FE draw, brownout / foldback and
 * the chain's warnings (hot wire, corona, missing lightning arrestor).
 */
public class AmplifierBlock extends BaseEntityBlock {
    public static final DirectionProperty FACING = BlockStateProperties.HORIZONTAL_FACING;

    private final AmpModel.Tier tier;
    private final VoxelShape shape;

    public AmplifierBlock(AmpModel.Tier tier, Properties properties) {
        super(properties);
        this.tier = tier;
        this.shape = switch (tier) {
            case W100 -> Block.box(2, 0, 2, 14, 8, 14);
            case KW1 -> Block.box(1, 0, 1, 15, 12, 15);
            case KW10 -> Block.box(0, 0, 0, 16, 16, 16);
        };
        registerDefaultState(stateDefinition.any().setValue(FACING, Direction.NORTH));
    }

    public AmpModel.Tier tier() {
        return tier;
    }

    @Override
    protected MapCodec<? extends BaseEntityBlock> codec() {
        return simpleCodec(p -> new AmplifierBlock(tier, p));
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
        return shape;
    }

    @Override
    protected RenderShape getRenderShape(BlockState state) {
        return RenderShape.MODEL;
    }

    @Override
    public @Nullable BlockEntity newBlockEntity(BlockPos pos, BlockState state) {
        return new AmplifierBlockEntity(pos, state);
    }

    @Override
    public @Nullable <T extends BlockEntity> BlockEntityTicker<T> getTicker(Level level, BlockState state, BlockEntityType<T> type) {
        return level.isClientSide() ? null : createTickerHelper(type, RadioAmpContent.AMPLIFIER_BE.get(), AmplifierBlockEntity::serverTick);
    }

    @Override
    public void setPlacedBy(Level level, BlockPos pos, BlockState state, @Nullable LivingEntity placer, ItemStack stack) {
        super.setPlacedBy(level, pos, state, placer, stack);
        if (!level.isClientSide() && placer instanceof Player p && level.getBlockEntity(pos) instanceof AmplifierBlockEntity be)
            be.setOwner(p.getUUID());
    }

    @Override
    protected InteractionResult useWithoutItem(BlockState state, Level level, BlockPos pos, Player player, BlockHitResult hit) {
        if (!level.isClientSide() && level.getBlockEntity(pos) instanceof AmplifierBlockEntity be) {
            ExciterLink link = ExciterLink.through(level, pos);
            List<String> lines = be.statusLines(link == null ? List.of("No exciter: connect an SDR (touching, or through coax)") : link.warnings());
            for (String l : lines) player.sendSystemMessage(Component.literal(l));
        }
        return InteractionResult.sidedSuccess(level.isClientSide());
    }
}
//?}
