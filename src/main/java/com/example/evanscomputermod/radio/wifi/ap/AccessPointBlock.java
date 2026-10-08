package com.example.evanscomputermod.radio.wifi.ap;

//? if <=1.21.1 {
import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.radio.RadioContent;
import com.mojang.serialization.MapCodec;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.network.chat.Component;
import net.minecraft.server.level.ServerPlayer;
import net.minecraft.world.InteractionResult;
import net.minecraft.world.SimpleMenuProvider;
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
import net.minecraft.world.level.block.state.properties.BooleanProperty;
import net.minecraft.world.level.block.state.properties.DirectionProperty;
import net.minecraft.world.phys.BlockHitResult;
import net.minecraft.world.phys.shapes.CollisionContext;
import net.minecraft.world.phys.shapes.Shapes;
import net.minecraft.world.phys.shapes.VoxelShape;
import org.jetbrains.annotations.Nullable;

/**
 * Wi-Fi Access Point: a small box with two antennas, placed on or next to a
 * network cable. Right-click opens settings and status (owner, op or creative
 * only); sneak + right-click with a wrench ({@code #evanscomputermod:rf_wrenches})
 * resets it to factory settings. {@link #ACTIVE} lights the status LED while
 * the AP is attached to a cable segment.
 */
public class AccessPointBlock extends BaseEntityBlock {
    public static final MapCodec<AccessPointBlock> CODEC = simpleCodec(AccessPointBlock::new);
    public static final DirectionProperty FACING = BlockStateProperties.HORIZONTAL_FACING;
    public static final BooleanProperty ACTIVE = BooleanProperty.create("active");
    private static final VoxelShape SHAPE = Shapes.or(box(2, 0, 3, 14, 4, 13), box(3, 4, 11, 4, 13, 12), box(12, 4, 11, 13, 13, 12));

    public AccessPointBlock(Properties properties) {
        super(properties);
        registerDefaultState(stateDefinition.any().setValue(FACING, Direction.NORTH).setValue(ACTIVE, false));
    }

    @Override
    protected MapCodec<? extends BaseEntityBlock> codec() {
        return CODEC;
    }

    @Override
    protected void createBlockStateDefinition(StateDefinition.Builder<Block, BlockState> builder) {
        builder.add(FACING, ACTIVE);
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
        return new AccessPointBlockEntity(pos, state);
    }

    @Override
    public @Nullable <T extends BlockEntity> BlockEntityTicker<T> getTicker(Level level, BlockState state, BlockEntityType<T> type) {
        return level.isClientSide() ? null
                : createTickerHelper(type, AccessPointContent.ACCESS_POINT_BE.get(), AccessPointBlockEntity::serverTick);
    }

    @Override
    public void setPlacedBy(Level level, BlockPos pos, BlockState state, @Nullable LivingEntity placer, ItemStack stack) {
        super.setPlacedBy(level, pos, state, placer, stack);
        if (!level.isClientSide() && placer instanceof Player p && level.getBlockEntity(pos) instanceof AccessPointBlockEntity be)
            be.claim(p);
    }

    @Override
    protected void neighborChanged(BlockState state, Level level, BlockPos pos, Block neighbor, BlockPos from, boolean moving) {
        super.neighborChanged(state, level, pos, neighbor, from, moving);
        if (!level.isClientSide() && level.getBlockEntity(pos) instanceof AccessPointBlockEntity be) be.markCableDirty();
    }

    @Override
    protected InteractionResult useWithoutItem(BlockState state, Level level, BlockPos pos, Player player, BlockHitResult hit) {
        if (!level.isClientSide() && player instanceof ServerPlayer sp && level.getBlockEntity(pos) instanceof AccessPointBlockEntity be)
            openSettings(sp, be);
        return InteractionResult.sidedSuccess(level.isClientSide());
    }

    /** Opens the settings screen for an allowed player; tells anyone else who owns it. Returns true if opened. */
    public static boolean openSettings(ServerPlayer player, AccessPointBlockEntity be) {
        if (!be.canConfigure(player)) {
            player.displayClientMessage(Component.translatable("message.evanscomputermod.access_point.not_owner",
                    be.ownerName() == null ? "?" : be.ownerName()), true);
            return false;
        }
        player.openMenu(new SimpleMenuProvider((id, inv, p) -> new AccessPointMenu(id, inv, be),
                Component.translatable("block.evanscomputermod.access_point")),
                buf -> ApPackets.writeView(buf, be.view(null)));
        return true;
    }

    /** Items that factory-reset the AP on sneak + right-click: the rf_wrenches tag, or the radio wrench item once it exists. */
    public static boolean isWrench(ItemStack stack) {
        if (stack.isEmpty()) return false;
        if (stack.is(RadioContent.RF_WRENCHES)) return true;
        return BuiltInRegistries.ITEM.getKey(stack.getItem()).equals(EvansComputerMod.id("rf_wrench"));
    }
}
//?}
