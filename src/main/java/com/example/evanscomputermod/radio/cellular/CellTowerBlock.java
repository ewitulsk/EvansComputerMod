package com.example.evanscomputermod.radio.cellular;

//? if <=1.21.1 {
import com.mojang.serialization.MapCodec;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.network.chat.Component;
import net.minecraft.server.level.ServerPlayer;
import net.minecraft.world.InteractionHand;
import net.minecraft.world.InteractionResult;
import net.minecraft.world.ItemInteractionResult;
import net.minecraft.world.SimpleMenuProvider;
import net.minecraft.world.entity.LivingEntity;
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
 * Cell Tower (base station): a cabinet at the foot of a mast. Cable it through
 * the RJ45 port underneath or on a side; stack mast blocks on top and mount LTE
 * / mmWave antennas on them. Right-click with a blank SIM card to issue a SIM
 * of the tower's carrier; right-click with an empty hand to open the carrier
 * screen (settings, issue SIMs, the carrier's SIMs with what is attached and
 * data limits). {@link #ACTIVE} lights the status lamp while it is on the air
 * and cabled.
 */
public class CellTowerBlock extends BaseEntityBlock implements com.example.evanscomputermod.api.network.CableConnectable {
    public static final MapCodec<CellTowerBlock> CODEC = simpleCodec(CellTowerBlock::new);
    public static final DirectionProperty FACING = BlockStateProperties.HORIZONTAL_FACING;
    public static final BooleanProperty ACTIVE = BooleanProperty.create("active");

    public CellTowerBlock(Properties properties) {
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

    /** RJ45 ports underneath and on the three sides that aren't the door; the top is the mast. */
    @Override
    public boolean acceptsCable(BlockState state, Direction face) {
        return face != Direction.UP && face != state.getValue(FACING);
    }

    @Override
    protected RenderShape getRenderShape(BlockState state) {
        return RenderShape.MODEL;
    }

    @Override
    public @Nullable BlockEntity newBlockEntity(BlockPos pos, BlockState state) {
        return new CellTowerBlockEntity(pos, state);
    }

    @Override
    public @Nullable <T extends BlockEntity> BlockEntityTicker<T> getTicker(Level level, BlockState state, BlockEntityType<T> type) {
        return level.isClientSide() ? null : createTickerHelper(type, CellularContent.CELL_TOWER_BE.get(), CellTowerBlockEntity::serverTick);
    }

    @Override
    public void setPlacedBy(Level level, BlockPos pos, BlockState state, @Nullable LivingEntity placer, ItemStack stack) {
        super.setPlacedBy(level, pos, state, placer, stack);
        if (!level.isClientSide() && placer instanceof Player p && level.getBlockEntity(pos) instanceof CellTowerBlockEntity be)
            be.claim(p);
    }

    @Override
    protected void neighborChanged(BlockState state, Level level, BlockPos pos, Block neighbor, BlockPos from, boolean moving) {
        super.neighborChanged(state, level, pos, neighbor, from, moving);
        if (!level.isClientSide() && level.getBlockEntity(pos) instanceof CellTowerBlockEntity be) be.markDirty();
    }

    /** A blank SIM card: the tower's carrier issues it. */
    @Override
    protected ItemInteractionResult useItemOn(ItemStack stack, BlockState state, Level level, BlockPos pos, Player player,
                                              InteractionHand hand, BlockHitResult hit) {
        if (!(stack.getItem() instanceof SimCardItem) || SimCardItem.isIssued(stack))
            return ItemInteractionResult.PASS_TO_DEFAULT_BLOCK_INTERACTION;
        if (!level.isClientSide() && level.getBlockEntity(pos) instanceof CellTowerBlockEntity be) issueFromHand(be, player, stack);
        return ItemInteractionResult.sidedSuccess(level.isClientSide());
    }

    /** Turn one blank SIM from {@code blank} into an issued one in the player's inventory; returns it (or null). */
    public static ItemStack issueFromHand(CellTowerBlockEntity be, Player player, ItemStack blank) {
        SimCard sim = be.issueSim(player, "");
        if (sim == null) {
            player.displayClientMessage(Component.literal(be.lastMessage() == null ? "Can't issue a SIM here" : be.lastMessage()), true);
            return null;
        }
        if (!player.getAbilities().instabuild) blank.shrink(1);
        ItemStack out = SimCardItem.issued(sim);
        if (!player.getInventory().add(out.copy())) player.drop(out.copy(), false);
        player.displayClientMessage(Component.translatable("message.evanscomputermod.cell_tower.issued", sim.carrier(), sim.imsiText()), true);
        return out;
    }

    @Override
    protected InteractionResult useWithoutItem(BlockState state, Level level, BlockPos pos, Player player, BlockHitResult hit) {
        if (!level.isClientSide() && player instanceof ServerPlayer sp && level.getBlockEntity(pos) instanceof CellTowerBlockEntity be)
            open(sp, be);
        return InteractionResult.sidedSuccess(level.isClientSide());
    }

    /** The carrier screen for the owner (op, creative); everyone else gets the status in chat. */
    public static boolean open(ServerPlayer player, CellTowerBlockEntity be) {
        if (!be.canConfigure(player)) {
            for (String line : be.statusLines()) player.sendSystemMessage(Component.literal(line));
            return false;
        }
        player.openMenu(new SimpleMenuProvider((id, inv, p) -> new CellTowerMenu(id, inv, be),
                        Component.translatable("block.evanscomputermod.cell_tower")),
                buf -> CellPackets.writeView(buf, CellPackets.view(be, null)));
        return true;
    }
}
//?}
