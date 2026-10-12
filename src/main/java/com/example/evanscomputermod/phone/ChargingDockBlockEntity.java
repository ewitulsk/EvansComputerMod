package com.example.evanscomputermod.phone;

//? if <=1.21.1 {
import com.example.evanscomputermod.energy.RadioEnergyStorage;
import net.minecraft.core.BlockPos;
import net.minecraft.core.HolderLookup;
import net.minecraft.nbt.CompoundTag;
import net.minecraft.network.chat.Component;
import net.minecraft.network.protocol.Packet;
import net.minecraft.network.protocol.game.ClientGamePacketListener;
import net.minecraft.network.protocol.game.ClientboundBlockEntityDataPacket;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.entity.BlockEntity;
import net.minecraft.world.level.block.state.BlockState;
import net.neoforged.neoforge.energy.IEnergyStorage;

/** The Charging Dock's phone and FE buffer; charges the docked phone's battery component every tick. */
public class ChargingDockBlockEntity extends BlockEntity {
    public static final int FE_CAPACITY = 100_000, FE_MAX_IN = 1_000;

    private ItemStack phone = ItemStack.EMPTY;
    private final RadioEnergyStorage energy = new RadioEnergyStorage(FE_CAPACITY, FE_MAX_IN, 0, this::setChanged);
    /** Fraction of a percent not yet written to the phone (charging in ticks smaller than 0.01 %). */
    private double carry;

    public ChargingDockBlockEntity(BlockPos pos, BlockState state) {
        super(PhoneContent.CHARGING_DOCK_BE.get(), pos, state);
    }

    public ItemStack phone() {
        return phone.copy();
    }

    /** The docked phone itself (tests read its battery). */
    public ItemStack phoneStack() {
        return phone;
    }

    public void setPhone(ItemStack stack) {
        phone = stack.isEmpty() ? ItemStack.EMPTY : stack.copyWithCount(1);
        carry = 0;
        setChanged();
        if (level != null && !level.isClientSide()) {
            level.sendBlockUpdated(worldPosition, getBlockState(), getBlockState(), Block.UPDATE_CLIENTS);
            updateLamp(level, worldPosition, getBlockState());
        }
    }

    public IEnergyStorage energy() {
        return energy;
    }

    public static void serverTick(Level level, BlockPos pos, BlockState state, ChargingDockBlockEntity be) {
        be.tick(level, pos, state);
    }

    private void tick(Level level, BlockPos pos, BlockState state) {
        if (!phone.isEmpty() && PhoneConfig.batteryEnabled()) {
            double now = PhoneItemData.batteryPercent(phone);
            if (now < 100) {
                double add = PhoneConfig.dockPerMinute() / 1200.0;
                int fePerPercent = PhoneConfig.dockFePerPercent();
                if (fePerPercent > 0) {
                    int want = (int) Math.ceil(add * fePerPercent);
                    int got = energy.consume(want);
                    add = want == 0 ? 0 : add * got / want;
                }
                carry += add;
                if (carry >= 0.01 || now + carry >= 100) {
                    PhoneItemData.setBatteryPercent(phone, now + carry);
                    carry = 0;
                    setChanged();
                    if (level.getGameTime() % 20 == 0) level.sendBlockUpdated(pos, state, state, Block.UPDATE_CLIENTS);
                }
            }
        }
        updateLamp(level, pos, state);
    }

    private void updateLamp(Level level, BlockPos pos, BlockState state) {
        boolean charging = !phone.isEmpty() && PhoneConfig.batteryEnabled() && PhoneItemData.batteryPercent(phone) < 100
                && (PhoneConfig.dockFePerPercent() == 0 || energy.getEnergyStored() > 0);
        if (state.getValue(ChargingDockBlock.ACTIVE) != charging) {
            level.setBlock(pos, state.setValue(ChargingDockBlock.ACTIVE, charging), Block.UPDATE_ALL);
        }
    }

    public Component status() {
        if (phone.isEmpty()) return Component.translatable("message.evanscomputermod.charging_dock.empty", energy.getEnergyStored());
        return Component.translatable("message.evanscomputermod.charging_dock.status",
                String.format(java.util.Locale.ROOT, "%.0f", PhoneItemData.batteryPercent(phone)), energy.getEnergyStored());
    }

    @Override
    protected void saveAdditional(CompoundTag tag, HolderLookup.Provider registries) {
        super.saveAdditional(tag, registries);
        if (!phone.isEmpty()) tag.put("Phone", phone.save(registries));
        energy.save(tag, "Energy");
    }

    @Override
    protected void loadAdditional(CompoundTag tag, HolderLookup.Provider registries) {
        super.loadAdditional(tag, registries);
        phone = tag.contains("Phone") ? ItemStack.parseOptional(registries, tag.getCompound("Phone")) : ItemStack.EMPTY;
        energy.load(tag, "Energy");
    }

    @Override
    public CompoundTag getUpdateTag(HolderLookup.Provider registries) {
        CompoundTag tag = new CompoundTag();
        if (!phone.isEmpty()) tag.put("Phone", phone.save(registries));
        return tag;
    }

    @Override
    public Packet<ClientGamePacketListener> getUpdatePacket() {
        return ClientboundBlockEntityDataPacket.create(this);
    }
}
//?}
