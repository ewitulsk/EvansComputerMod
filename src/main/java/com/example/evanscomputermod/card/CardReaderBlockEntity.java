package com.example.evanscomputermod.card;

//? if <=1.21.1 {
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.core.HolderLookup;
import net.minecraft.nbt.CompoundTag;
import net.minecraft.network.chat.Component;
import net.minecraft.world.MenuProvider;
import net.minecraft.world.entity.player.Inventory;
import net.minecraft.world.entity.player.Player;
import net.minecraft.world.inventory.AbstractContainerMenu;
import net.minecraft.world.inventory.ContainerData;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.block.HopperBlock;
import net.minecraft.world.level.block.entity.BlockEntity;
import net.minecraft.world.level.block.state.BlockState;
import net.neoforged.neoforge.capabilities.Capabilities;
import net.neoforged.neoforge.items.IItemHandler;
import net.neoforged.neoforge.items.ItemHandlerHelper;
import net.neoforged.neoforge.items.ItemStackHandler;
import org.jetbrains.annotations.Nullable;

/**
 * The Card Reader/Writer's state: one card slot, the "Auto Eject Sim" setting and
 * whether the computer is done with the card ("released").
 *
 * <h2>Card life cycle</h2>
 * <ol>
 *   <li>A card arrives (right-click, hopper or pipe into any side): the computer gets
 *       {@code card_inserted}. The card is now <em>held</em>: automation can't pull it
 *       out, so a hopper underneath doesn't take a card before the computer has used it.
 *       A player can always take it by hand.</li>
 *   <li>The computer <em>releases</em> it: {@code eject()}, or a successful
 *       {@code write(...)} (writing is the end of a card writer's job).</li>
 *   <li>Released, with Auto Eject Sim = true: the reader pushes the card into the first
 *       adjacent inventory that takes it (down, north, south, west, east, up; a hopper
 *       that feeds the reader is skipped). If none takes it, it stays, released, and the
 *       push is retried every second (so a chest placed later fills).
 *       With Auto Eject Sim = false: the card waits, released, to be pulled out (hopper
 *       below, pipe extraction) or taken by hand.</li>
 *   <li>The card leaves: {@code card_removed}. The next card starts held again.</li>
 * </ol>
 */
public class CardReaderBlockEntity extends BlockEntity implements MenuProvider, CardHolder {
    private static final Direction[] PUSH_ORDER = {Direction.DOWN, Direction.NORTH, Direction.SOUTH, Direction.WEST, Direction.EAST, Direction.UP};
    private static final int RETRY_TICKS = 20;

    private final ItemStackHandler slot = new ItemStackHandler(1) {
        @Override
        public int getSlotLimit(int s) {
            return 1;
        }

        @Override
        public boolean isItemValid(int s, ItemStack stack) {
            return isCard(stack);
        }

        @Override
        protected void onContentsChanged(int s) {
            contentsChanged();
        }
    };

    /** What hoppers and pipes see: insert a card when empty; extract only once released. */
    private final IItemHandler automation = new IItemHandler() {
        @Override
        public int getSlots() {
            return 1;
        }

        @Override
        public ItemStack getStackInSlot(int s) {
            return slot.getStackInSlot(s);
        }

        @Override
        public ItemStack insertItem(int s, ItemStack stack, boolean simulate) {
            return slot.insertItem(s, stack, simulate);
        }

        @Override
        public ItemStack extractItem(int s, int amount, boolean simulate) {
            return released ? slot.extractItem(s, amount, simulate) : ItemStack.EMPTY;
        }

        @Override
        public int getSlotLimit(int s) {
            return 1;
        }

        @Override
        public boolean isItemValid(int s, ItemStack stack) {
            return isCard(stack);
        }
    };

    private final CardReaderPeripheral peripheral = new CardReaderPeripheral(this);
    /** Client side: what the renderer draws (the card's kind and faces only). */
    private ItemStack clientCard = ItemStack.EMPTY;
    private boolean autoEject;
    private boolean released;
    private boolean hadCard;
    private int retry;

    /** Synced to an open screen: 0 auto eject, 1 card state (see {@link #cardState}), 2 released, 3 kind ordinal + 1. */
    private final ContainerData data = new ContainerData() {
        @Override
        public int get(int i) {
            return switch (i) {
                case 0 -> autoEject ? 1 : 0;
                case 1 -> cardState();
                case 2 -> released ? 1 : 0;
                case 3 -> cardKindOrdinal() + 1;
                default -> 0;
            };
        }

        @Override
        public void set(int i, int v) {
        }

        @Override
        public int getCount() {
            return 4;
        }
    };

    public CardReaderBlockEntity(BlockPos pos, BlockState state) {
        super(CardContent.CARD_READER_BE.get(), pos, state);
    }

    /** Cards this reader takes: every {@link CardItem} and the {@code evanscomputermod:cards} item tag. */
    public static boolean isCard(ItemStack stack) {
        return CardItem.isCard(stack);
    }

    public IItemHandler automation() {
        return automation;
    }

    public CardReaderPeripheral peripheral() {
        return peripheral;
    }

    @Override
    public ItemStack card() {
        return slot.getStackInSlot(0);
    }

    @Override
    public boolean hasCard() {
        return !card().isEmpty();
    }

    @Override
    public boolean autoEject() {
        return autoEject;
    }

    @Override
    public boolean released() {
        return released;
    }

    /** 0 no card, 1 blank card (no applet written), 2 written card, 3 another item. */
    public int cardState() {
        ItemStack c = card();
        if (c.isEmpty()) return 0;
        if (CardItem.kindOf(c) == null) return 3;
        return CardItem.allBlank(c) ? 1 : 2;
    }

    /** The kind of the card in the slot as an ordinal for the screen (-1 none). */
    public int cardKindOrdinal() {
        CardKind k = CardItem.kindOf(card());
        return k == null ? -1 : k.ordinal();
    }

    public void setAutoEject(boolean on) {
        if (autoEject == on) return;
        autoEject = on;
        retry = 0;
        setChanged();
    }

    void insertByHand(ItemStack one) {
        slot.insertItem(0, one, false);
    }

    /** Take the card out (a player's hand, or the block breaking). */
    ItemStack takeByHand() {
        return slot.extractItem(0, 1, false);
    }

    /**
     * The computer is done with the card. With Auto Eject Sim it is pushed out now
     * (true if it left); otherwise it waits to be pulled.
     */
    @Override
    public boolean release() {
        if (!hasCard()) return false;
        released = true;
        setChanged();
        return autoEject && push();
    }

    /** Mark that the card was changed in place (written). */
    @Override
    public void cardChanged() {
        setChanged();
        syncToClients();
    }

    /** The card as clients see it: its item (kind), painted faces and whether its stripe is scrambled; nothing else. */
    public static ItemStack clientView(ItemStack card) {
        if (card.isEmpty() || CardItem.kindOf(card) == null) return ItemStack.EMPTY;
        ItemStack view = new ItemStack(card.getItem());
        CardFaces f = CardItem.faces(card);
        if (f != null) view.set(CardContent.CARD_FACES.get(), f);
        Stripe s = CardItem.stripe(card);
        if (s != null && s.corrupted()) view.set(CardContent.CARD_STRIPE.get(), new Stripe("", "", "", true));
        return view;
    }

    /** What the client renders in the slot (EMPTY with no card). */
    public ItemStack clientCard() {
        return level != null && level.isClientSide() ? clientCard : clientView(card());
    }

    private void syncToClients() {
        if (level != null && !level.isClientSide()) level.sendBlockUpdated(worldPosition, getBlockState(), getBlockState(), 3);
    }

    private void contentsChanged() {
        setChanged();
        if (level == null || level.isClientSide()) return;
        syncToClients();
        boolean has = hasCard();
        if (has != hadCard) {
            hadCard = has;
            released = false;
            retry = 0;
            BlockState s = getBlockState();
            if (s.hasProperty(CardReaderBlock.CARD) && s.getValue(CardReaderBlock.CARD) != has)
                level.setBlock(worldPosition, s.setValue(CardReaderBlock.CARD, has), 3);
            level.updateNeighbourForOutputSignal(worldPosition, getBlockState().getBlock());
            if (has) peripheral.cardInserted(card());
            else peripheral.cardRemoved();
        }
    }

    public static void serverTick(Level level, BlockPos pos, BlockState state, CardReaderBlockEntity be) {
        if (be.released && be.autoEject && be.hasCard() && ++be.retry >= RETRY_TICKS) {
            be.retry = 0;
            be.push();
        }
    }

    /** Push the card into the first adjacent inventory that takes it; true if it left. */
    private boolean push() {
        if (level == null || !hasCard()) return false;
        for (Direction d : PUSH_ORDER) {
            BlockPos n = worldPosition.relative(d);
            BlockState ns = level.getBlockState(n);
            // A hopper pointing into us is our feeder: pushing back would loop the card.
            if (ns.getBlock() instanceof HopperBlock && ns.getValue(HopperBlock.FACING) == d.getOpposite()) continue;
            IItemHandler target = level.getCapability(Capabilities.ItemHandler.BLOCK, n, d.getOpposite());
            if (target == null || target == automation) continue;
            ItemStack card = card().copy();
            if (!ItemHandlerHelper.insertItem(target, card, true).isEmpty()) continue;
            ItemStack out = slot.extractItem(0, 1, false);
            ItemStack rest = ItemHandlerHelper.insertItem(target, out, false);
            if (!rest.isEmpty()) slot.setStackInSlot(0, rest);   // changed its mind: keep it
            else return true;
        }
        return false;
    }

    // ------------------------------------------------------------ screen

    @Override
    public Component getDisplayName() {
        return Component.translatable("block.evanscomputermod.card_reader");
    }

    @Override
    public @Nullable AbstractContainerMenu createMenu(int id, Inventory inv, Player player) {
        return new CardReaderMenu(id, inv, this, data);
    }

    IItemHandler slotHandler() {
        return slot;
    }

    // ------------------------------------------------------------ persistence

    @Override
    protected void saveAdditional(CompoundTag tag, HolderLookup.Provider registries) {
        super.saveAdditional(tag, registries);
        tag.put("Card", slot.serializeNBT(registries));
        tag.putBoolean("AutoEject", autoEject);
        tag.putBoolean("Released", released);
    }

    @Override
    protected void loadAdditional(CompoundTag tag, HolderLookup.Provider registries) {
        super.loadAdditional(tag, registries);
        slot.deserializeNBT(registries, tag.getCompound("Card"));
        autoEject = tag.getBoolean("AutoEject");
        released = tag.getBoolean("Released");
        hadCard = hasCard();
    }

    /**
     * Clients get only the card's look ({@link #clientView}): the card itself, with its applets
     * and secrets, stays on the server; the block state shows whether one is in.
     */
    @Override
    public CompoundTag getUpdateTag(HolderLookup.Provider registries) {
        CompoundTag tag = new CompoundTag();
        ItemStack view = clientView(card());
        if (!view.isEmpty()) tag.put("ClientCard", view.save(registries));
        return tag;
    }

    @Override
    public net.minecraft.network.protocol.Packet<net.minecraft.network.protocol.game.ClientGamePacketListener> getUpdatePacket() {
        return net.minecraft.network.protocol.game.ClientboundBlockEntityDataPacket.create(this);
    }

    @Override
    public void handleUpdateTag(CompoundTag tag, HolderLookup.Provider registries) {
        clientCard = tag.contains("ClientCard") ? ItemStack.parseOptional(registries, tag.getCompound("ClientCard")) : ItemStack.EMPTY;
    }

    @Override
    public void onDataPacket(net.minecraft.network.Connection net, net.minecraft.network.protocol.game.ClientboundBlockEntityDataPacket pkt,
                             HolderLookup.Provider registries) {
        handleUpdateTag(pkt.getTag() == null ? new CompoundTag() : pkt.getTag(), registries);
    }
}
//?}
