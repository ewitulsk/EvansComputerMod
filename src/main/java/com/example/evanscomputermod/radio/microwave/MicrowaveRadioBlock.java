package com.example.evanscomputermod.radio.microwave;

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
import net.minecraft.world.phys.shapes.VoxelShape;
import org.jetbrains.annotations.Nullable;

import java.util.Map;

/**
 * Microwave radio (outdoor unit): joins the network cable it touches as a
 * bridge port and feeds the dish next to it. Right click: status. Sneak +
 * right click: next channel (wrapping into the next band). Band, width and
 * power are set from a computer ({@code microwave_radio} peripheral).
 */
public class MicrowaveRadioBlock extends BaseEntityBlock implements com.example.evanscomputermod.api.network.CableConnectable {
    public static final MapCodec<MicrowaveRadioBlock> CODEC = simpleCodec(MicrowaveRadioBlock::new);
    public static final DirectionProperty FACING = BlockStateProperties.HORIZONTAL_FACING;
    private static final VoxelShape SHAPE = Block.box(2, 0, 2, 14, 12, 14);

    public MicrowaveRadioBlock(Properties properties) {
        super(properties);
        registerDefaultState(stateDefinition.any().setValue(FACING, Direction.NORTH));
    }

    @Override
    protected MapCodec<? extends BaseEntityBlock> codec() {
        return CODEC;
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
        return new MicrowaveRadioBlockEntity(pos, state);
    }

    @Override
    public @Nullable <T extends BlockEntity> BlockEntityTicker<T> getTicker(Level level, BlockState state, BlockEntityType<T> type) {
        return level.isClientSide() ? null : createTickerHelper(type, MicrowaveContent.RADIO_BE.get(), MicrowaveRadioBlockEntity::serverTick);
    }

    @Override
    protected void neighborChanged(BlockState state, Level level, BlockPos pos, Block neighbor, BlockPos from, boolean moving) {
        super.neighborChanged(state, level, pos, neighbor, from, moving);
        if (!level.isClientSide() && level.getBlockEntity(pos) instanceof MicrowaveRadioBlockEntity be) be.markCableDirty();
    }

    @Override
    protected InteractionResult useWithoutItem(BlockState state, Level level, BlockPos pos, Player player, BlockHitResult hit) {
        if (level.isClientSide()) return InteractionResult.SUCCESS;
        if (!(level.getBlockEntity(pos) instanceof MicrowaveRadioBlockEntity be)) return InteractionResult.PASS;
        MicrowaveLink link = be.link();
        if (player.isShiftKeyDown()) {
            int next = link.channelNumber() + 1;
            if (next < link.band().channelCount(link.widthMhz())) link.setChannelNumber(next);
            else {
                MwBand[] bands = MwBand.values();
                MwBand b = bands[(link.band().ordinal() + 1) % bands.length];
                link.configure(b, b.defaultWidthMhz, 0);
            }
            be.setChanged();
        }
        Map<String, Object> s = link.status();
        Object linked = s.get("linked");
        player.displayClientMessage(Component.translatable("message.evanscomputermod.microwave_radio.status",
                link.band().ghz, link.channelNumber(), link.widthMhz(),
                Boolean.TRUE.equals(s.get("dish")) ? Component.translatable("message.evanscomputermod.microwave_radio.dish")
                        : Component.translatable("message.evanscomputermod.microwave_radio.no_dish"),
                Boolean.TRUE.equals(linked)
                        ? String.format("%.1f dBm, %s, %.0f Mbit/s", (Double) s.get("rssi_dbm"), s.get("modulation"), (Double) s.get("rate_mbps"))
                        : Component.translatable("message.evanscomputermod.microwave_radio.no_link")), true);
        return InteractionResult.CONSUME;
    }
}
//?}
