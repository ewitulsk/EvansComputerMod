package com.example.evanscomputermod.card.printer;

//? if <=1.21.1 {
import com.example.evanscomputermod.card.CardContent;
import com.example.evanscomputermod.card.CardFaces;
import com.example.evanscomputermod.card.CardItem;
import com.example.evanscomputermod.card.CardKind;
import com.example.evanscomputermod.card.Stripe;
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
import net.minecraft.world.item.DyeColor;
import net.minecraft.world.item.DyeItem;
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

import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

/**
 * The Card Printer's state: a card in, a card out, sixteen dye slots (one per dye colour),
 * the job in progress and the Auto Eject setting. {@link #print} takes a validated design,
 * checks the dyes it costs, consumes them and starts a job; {@link #PRINT_TICKS} later the
 * card gets its faces, fields and stripe and moves to the output slot, where Auto Eject
 * pushes it into an adjacent inventory (the reader's rules) or it waits to be taken.
 * The printer never touches an applet or a secret.
 */
public class CardPrinterBlockEntity extends BlockEntity implements MenuProvider {
    public static final int SLOT_IN = 0, SLOT_OUT = 1, SLOT_DYE0 = 2, SLOTS = 18;
    public static final int PRINT_TICKS = 40;
    private static final Direction[] PUSH_ORDER = {Direction.DOWN, Direction.NORTH, Direction.SOUTH, Direction.WEST, Direction.EAST, Direction.UP};
    private static final int RETRY_TICKS = 20;

    private final ItemStackHandler slots = new ItemStackHandler(SLOTS) {
        @Override
        public int getSlotLimit(int s) {
            return s < SLOT_DYE0 ? 1 : 64;
        }

        @Override
        public boolean isItemValid(int s, ItemStack stack) {
            if (s == SLOT_IN) return CardItem.isCard(stack);
            if (s == SLOT_OUT) return false;
            return stack.getItem() instanceof DyeItem d && d.getDyeColor().ordinal() == s - SLOT_DYE0;
        }

        @Override
        protected void onContentsChanged(int s) {
            contentsChanged(s);
        }
    };

    private final CardPrinterPeripheral peripheral = new CardPrinterPeripheral(this);
    private boolean autoEject;
    private int progress = -1;
    private CardFaces jobFaces;
    private Map<String, String> jobFields;
    private Stripe jobStripe;
    private boolean hadIn;
    private int retry;

    /** Synced to an open screen: 0 auto eject, 1 progress (-1 idle), 2 in-card kind + 1, 3 out-card kind + 1. */
    private final ContainerData data = new ContainerData() {
        @Override
        public int get(int i) {
            return switch (i) {
                case 0 -> autoEject ? 1 : 0;
                case 1 -> progress;
                case 2 -> kindOrdinal(SLOT_IN) + 1;
                case 3 -> kindOrdinal(SLOT_OUT) + 1;
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

    public CardPrinterBlockEntity(BlockPos pos, BlockState state) {
        super(CardContent.CARD_PRINTER_BE.get(), pos, state);
    }

    private int kindOrdinal(int slot) {
        CardKind k = CardItem.kindOf(slots.getStackInSlot(slot));
        return k == null ? -1 : k.ordinal();
    }

    public CardPrinterPeripheral peripheral() {
        return peripheral;
    }

    public ItemStack cardIn() {
        return slots.getStackInSlot(SLOT_IN);
    }

    public ItemStack cardOut() {
        return slots.getStackInSlot(SLOT_OUT);
    }

    public boolean printing() {
        return progress >= 0;
    }

    public int progress() {
        return progress;
    }

    public boolean autoEject() {
        return autoEject;
    }

    public void setAutoEject(boolean on) {
        if (autoEject == on) return;
        autoEject = on;
        retry = 0;
        setChanged();
    }

    /** Dye counts by colour name. */
    public Map<String, Integer> dyes() {
        Map<String, Integer> m = new LinkedHashMap<>();
        for (DyeColor c : DyeColor.values()) {
            int n = slots.getStackInSlot(SLOT_DYE0 + c.ordinal()).getCount();
            if (n > 0) m.put(c.getName(), n);
        }
        return m;
    }

    public int comparator() {
        return cardOut().isEmpty() ? 0 : 15;
    }

    IItemHandler slotHandler() {
        return slots;
    }

    // ------------------------------------------------------------ hands and automation

    /** A card or dye from a player's hand; returns what wasn't taken. */
    ItemStack insertByHand(ItemStack stack) {
        if (CardItem.isCard(stack)) return cardIn().isEmpty() && !printing() ? slots.insertItem(SLOT_IN, stack, false) : stack;
        if (stack.getItem() instanceof DyeItem d) return slots.insertItem(SLOT_DYE0 + d.getDyeColor().ordinal(), stack, false);
        return stack;
    }

    /** The printed card, else the unprinted one (not while printing), else EMPTY. */
    ItemStack takeByHand() {
        if (!cardOut().isEmpty()) return slots.extractItem(SLOT_OUT, 1, false);
        if (!cardIn().isEmpty() && !printing()) return slots.extractItem(SLOT_IN, 1, false);
        return ItemStack.EMPTY;
    }

    List<ItemStack> drops() {
        List<ItemStack> l = new ArrayList<>();
        for (int i = 0; i < SLOTS; i++) {
            ItemStack s = slots.getStackInSlot(i);
            if (!s.isEmpty()) l.add(s);
        }
        return l;
    }

    /** Sides and top: insert cards and dye. Below: extract the printed card. */
    public IItemHandler automation(@Nullable Direction side) {
        return new IItemHandler() {
            @Override
            public int getSlots() {
                return SLOTS;
            }

            @Override
            public ItemStack getStackInSlot(int s) {
                return slots.getStackInSlot(s);
            }

            @Override
            public ItemStack insertItem(int s, ItemStack stack, boolean simulate) {
                if (side == Direction.DOWN || s == SLOT_OUT) return stack;
                if (s == SLOT_IN && printing()) return stack;
                return slots.insertItem(s, stack, simulate);
            }

            @Override
            public ItemStack extractItem(int s, int amount, boolean simulate) {
                return s == SLOT_OUT ? slots.extractItem(s, amount, simulate) : ItemStack.EMPTY;
            }

            @Override
            public int getSlotLimit(int s) {
                return slots.getSlotLimit(s);
            }

            @Override
            public boolean isItemValid(int s, ItemStack stack) {
                return slots.isItemValid(s, stack);
            }
        };
    }

    private void contentsChanged(int slot) {
        setChanged();
        if (level == null || level.isClientSide()) return;
        if (slot == SLOT_IN) {
            boolean has = !cardIn().isEmpty();
            if (has != hadIn) {
                hadIn = has;
                if (has) peripheral.cardInserted(cardIn());
                else peripheral.cardRemoved();
            }
        }
        if (slot == SLOT_OUT) {
            retry = 0;
            level.updateNeighbourForOutputSignal(worldPosition, getBlockState().getBlock());
        }
    }

    // ------------------------------------------------------------ the job

    /** The dye each palette colour used by the design costs: {colour name: count}. */
    public static Map<String, Integer> cost(CardFaces faces, CardKind kind) {
        boolean[] used = new boolean[faces.palette().length];
        int n = kind.texels();
        boolean[] mf = kind.mask(true), mb = kind.mask(false);
        for (int i = 0; i < n; i++) {
            if (!mf[i]) used[Math.min(CardFaces.texel(faces.front(), i), used.length - 1)] = true;
            if (!mb[i]) used[Math.min(CardFaces.texel(faces.back(), i), used.length - 1)] = true;
        }
        Map<String, Integer> m = new LinkedHashMap<>();
        for (int i = 0; i < used.length; i++) if (used[i]) m.merge(nearestDye(faces.palette()[i]).getName(), 1, Integer::sum);
        return m;
    }

    /** The vanilla dye nearest an RGB colour. */
    public static DyeColor nearestDye(int rgb) {
        int r = rgb >> 16 & 255, g = rgb >> 8 & 255, b = rgb & 255;
        DyeColor best = DyeColor.WHITE;
        long bd = Long.MAX_VALUE;
        for (DyeColor c : DyeColor.values()) {
            int t = c.getTextureDiffuseColor();
            int dr = r - (t >> 16 & 255), dg = g - (t >> 8 & 255), db = b - (t & 255);
            long d = (long) dr * dr + (long) dg * dg + (long) db * db;
            if (d < bd) {
                bd = d;
                best = c;
            }
        }
        return best;
    }

    /** Dyes the printer lacks for {@code cost}: {colour: missing count}. */
    public Map<String, Integer> missing(Map<String, Integer> cost) {
        Map<String, Integer> m = new LinkedHashMap<>();
        for (Map.Entry<String, Integer> e : cost.entrySet()) {
            DyeColor c = DyeColor.byName(e.getKey(), DyeColor.WHITE);
            int have = slots.getStackInSlot(SLOT_DYE0 + c.ordinal()).getCount();
            if (have < e.getValue()) m.put(e.getKey(), e.getValue() - have);
        }
        return m;
    }

    /** Start printing the card in the input slot. Returns null, or why not. */
    public @Nullable String print(CardFaces faces, Map<String, String> fields, @Nullable Stripe stripe) {
        ItemStack card = cardIn();
        if (card.isEmpty()) return "no card in the printer";
        if (printing()) return "already printing";
        if (!cardOut().isEmpty()) return "the output slot is full";
        CardKind k = CardItem.kindOf(card);
        if (k == null) return "not a card";
        if (!faces.fits(k)) return "the design is " + k.faceWidth + "x" + k.faceHeight + " texels per face for a " + k.id() + " card";
        if (stripe != null && !k.stripe) return "a " + k.id() + " card has no stripe";
        Map<String, Integer> cost = cost(faces, k);
        Map<String, Integer> miss = missing(cost);
        if (!miss.isEmpty()) {
            StringBuilder sb = new StringBuilder();
            miss.forEach((c, n) -> sb.append(sb.isEmpty() ? "" : " ").append("no_dye:").append(c));
            return sb.toString();
        }
        for (Map.Entry<String, Integer> e : cost.entrySet())
            slots.extractItem(SLOT_DYE0 + DyeColor.byName(e.getKey(), DyeColor.WHITE).ordinal(), e.getValue(), false);
        jobFaces = faces;
        jobFields = fields;
        jobStripe = stripe;
        progress = 0;
        setActive(true);
        setChanged();
        return null;
    }

    private void setActive(boolean on) {
        BlockState s = getBlockState();
        if (level != null && s.hasProperty(CardPrinterBlock.ACTIVE) && s.getValue(CardPrinterBlock.ACTIVE) != on)
            level.setBlock(worldPosition, s.setValue(CardPrinterBlock.ACTIVE, on), 3);
    }

    private void finish() {
        ItemStack card = slots.extractItem(SLOT_IN, 1, false);
        progress = -1;
        setActive(false);
        if (card.isEmpty()) {
            peripheral.printFailed("the card was taken out");
            return;
        }
        card.set(CardContent.CARD_FACES.get(), jobFaces);
        if (jobFields != null && !jobFields.isEmpty()) card.set(CardContent.CARD_PRINT.get(), jobFields);
        if (jobStripe != null) card.set(CardContent.CARD_STRIPE.get(), jobStripe);
        slots.setStackInSlot(SLOT_OUT, card);
        jobFaces = null;
        jobFields = null;
        jobStripe = null;
        setChanged();
        peripheral.printDone();
        if (autoEject) push();
    }

    /** Done with the printed card: Auto Eject pushes it out now (true if it left). */
    public boolean release() {
        return !cardOut().isEmpty() && autoEject && push();
    }

    public static void serverTick(Level level, BlockPos pos, BlockState state, CardPrinterBlockEntity be) {
        if (be.progress >= 0 && ++be.progress >= PRINT_TICKS) be.finish();
        if (be.autoEject && !be.cardOut().isEmpty() && ++be.retry >= RETRY_TICKS) {
            be.retry = 0;
            be.push();
        }
    }

    private boolean push() {
        if (level == null || cardOut().isEmpty()) return false;
        for (Direction d : PUSH_ORDER) {
            BlockPos n = worldPosition.relative(d);
            BlockState ns = level.getBlockState(n);
            if (ns.getBlock() instanceof HopperBlock && ns.getValue(HopperBlock.FACING) == d.getOpposite()) continue;
            IItemHandler target = level.getCapability(Capabilities.ItemHandler.BLOCK, n, d.getOpposite());
            if (target == null) continue;
            if (!ItemHandlerHelper.insertItem(target, cardOut().copy(), true).isEmpty()) continue;
            ItemStack out = slots.extractItem(SLOT_OUT, 1, false);
            ItemStack rest = ItemHandlerHelper.insertItem(target, out, false);
            if (!rest.isEmpty()) slots.setStackInSlot(SLOT_OUT, rest);
            else return true;
        }
        return false;
    }

    // ------------------------------------------------------------ screen

    @Override
    public Component getDisplayName() {
        return Component.translatable("block.evanscomputermod.card_printer");
    }

    @Override
    public @Nullable AbstractContainerMenu createMenu(int id, Inventory inv, Player player) {
        return new CardPrinterMenu(id, inv, this, data);
    }

    // ------------------------------------------------------------ persistence

    @Override
    protected void saveAdditional(CompoundTag tag, HolderLookup.Provider registries) {
        super.saveAdditional(tag, registries);
        tag.put("Slots", slots.serializeNBT(registries));
        tag.putBoolean("AutoEject", autoEject);
        tag.putInt("Progress", progress);
        if (jobFaces != null) {
            CardFaces.CODEC.encodeStart(net.minecraft.nbt.NbtOps.INSTANCE, jobFaces).result().ifPresent(t -> tag.put("JobFaces", t));
            CompoundTag f = new CompoundTag();
            if (jobFields != null) jobFields.forEach(f::putString);
            tag.put("JobFields", f);
            if (jobStripe != null) Stripe.CODEC.encodeStart(net.minecraft.nbt.NbtOps.INSTANCE, jobStripe).result().ifPresent(t -> tag.put("JobStripe", t));
        }
    }

    @Override
    protected void loadAdditional(CompoundTag tag, HolderLookup.Provider registries) {
        super.loadAdditional(tag, registries);
        slots.deserializeNBT(registries, tag.getCompound("Slots"));
        autoEject = tag.getBoolean("AutoEject");
        progress = tag.contains("Progress") ? tag.getInt("Progress") : -1;
        hadIn = !cardIn().isEmpty();
        jobFaces = tag.contains("JobFaces") ? CardFaces.CODEC.parse(net.minecraft.nbt.NbtOps.INSTANCE, tag.get("JobFaces")).result().orElse(null) : null;
        if (jobFaces == null) progress = -1;
        else {
            CompoundTag f = tag.getCompound("JobFields");
            jobFields = new LinkedHashMap<>();
            for (String k : f.getAllKeys()) jobFields.put(k, f.getString(k));
            jobStripe = tag.contains("JobStripe") ? Stripe.CODEC.parse(net.minecraft.nbt.NbtOps.INSTANCE, tag.get("JobStripe")).result().orElse(null) : null;
        }
    }

    /** Nothing for clients: cards (and their secrets) stay on the server; the block state shows activity. */
    @Override
    public CompoundTag getUpdateTag(HolderLookup.Provider registries) {
        return new CompoundTag();
    }
}
//?}
