package com.example.evanscomputermod.radio.power;

//? if <=1.21.1 {
import com.example.evanscomputermod.energy.RadioEnergyStorage;
import com.example.evanscomputermod.radio.RadioConfig;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.core.HolderLookup;
import net.minecraft.nbt.CompoundTag;
import net.minecraft.network.chat.Component;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.item.crafting.RecipeType;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.block.entity.BlockEntity;
import net.minecraft.world.level.block.state.BlockState;
import net.neoforged.neoforge.capabilities.Capabilities;
import net.neoforged.neoforge.energy.IEnergyStorage;
import net.neoforged.neoforge.items.ItemStackHandler;

/**
 * Burner Generator state: one fuel slot, a burn timer and an FE buffer. Fuel
 * value comes from {@code ItemStack#getBurnTime(RecipeType.SMELTING)} (the
 * furnace-fuel data map). Disabled by server config: it keeps its fuel and
 * energy but produces nothing.
 */
public class BurnerGeneratorBlockEntity extends BlockEntity {

    public static final int CAPACITY = 40_000;
    public static final int MAX_PUSH = 400;

    private final ItemStackHandler fuel = new ItemStackHandler(1) {
        @Override
        public boolean isItemValid(int slot, ItemStack stack) {
            return isFuel(stack);
        }

        @Override
        protected void onContentsChanged(int slot) {
            setChanged();
        }
    };
    private final RadioEnergyStorage energy = new RadioEnergyStorage(CAPACITY, 0, MAX_PUSH, this::setChanged);
    private int burnTicks;
    private int burnTotal;

    public BurnerGeneratorBlockEntity(BlockPos pos, BlockState state) {
        super(RadioPowerContent.BURNER_GENERATOR_BE.get(), pos, state);
    }

    public static boolean isFuel(ItemStack stack) {
        return !stack.isEmpty() && stack.getBurnTime(RecipeType.SMELTING) > 0;
    }

    public ItemStackHandler fuel() {
        return fuel;
    }

    public IEnergyStorage energy() {
        return energy;
    }

    public int burnTicks() {
        return burnTicks;
    }

    public static void serverTick(Level level, BlockPos pos, BlockState state, BurnerGeneratorBlockEntity be) {
        be.tick(level, pos, state);
    }

    private void tick(Level level, BlockPos pos, BlockState state) {
        boolean enabled = RadioConfig.burnerGeneratorEnabled();
        if (enabled) {
            if (burnTicks <= 0 && energy.getEnergyStored() < CAPACITY) {
                ItemStack stack = fuel.getStackInSlot(0);
                int value = isFuel(stack) ? stack.getBurnTime(RecipeType.SMELTING) : 0;
                if (value > 0) {
                    ItemStack remainder = stack.getCraftingRemainingItem();
                    fuel.extractItem(0, 1, false);
                    if (!remainder.isEmpty() && fuel.getStackInSlot(0).isEmpty()) fuel.setStackInSlot(0, remainder);
                    burnTicks = burnTotal = value;
                    setChanged();
                }
            }
            if (burnTicks > 0) {
                burnTicks--;
                energy.generate(RadioConfig.burnerFePerTick());
            }
        }
        boolean lit = enabled && burnTicks > 0;
        if (state.getValue(BurnerGeneratorBlock.LIT) != lit)
            level.setBlock(pos, state.setValue(BurnerGeneratorBlock.LIT, lit), 3);
        pushEnergy(level, pos);
    }

    private void pushEnergy(Level level, BlockPos pos) {
        for (Direction d : Direction.values()) {
            if (energy.getEnergyStored() <= 0) return;
            IEnergyStorage target = level.getCapability(Capabilities.EnergyStorage.BLOCK, pos.relative(d), d.getOpposite());
            if (target == null || target == energy || !target.canReceive()) continue;
            int sent = target.receiveEnergy(Math.min(MAX_PUSH, energy.getEnergyStored()), false);
            energy.consume(sent);
        }
    }

    public Component status() {
        if (!RadioConfig.burnerGeneratorEnabled())
            return Component.translatable("message.evanscomputermod.burner_generator.disabled");
        return Component.translatable("message.evanscomputermod.burner_generator.status",
                energy.getEnergyStored(), CAPACITY, burnTicks / 20, RadioConfig.burnerFePerTick());
    }

    @Override
    protected void saveAdditional(CompoundTag tag, HolderLookup.Provider registries) {
        super.saveAdditional(tag, registries);
        tag.put("Fuel", fuel.serializeNBT(registries));
        energy.save(tag, "Energy");
        tag.putInt("Burn", burnTicks);
        tag.putInt("BurnTotal", burnTotal);
    }

    @Override
    protected void loadAdditional(CompoundTag tag, HolderLookup.Provider registries) {
        super.loadAdditional(tag, registries);
        fuel.deserializeNBT(registries, tag.getCompound("Fuel"));
        energy.load(tag, "Energy");
        burnTicks = tag.getInt("Burn");
        burnTotal = tag.getInt("BurnTotal");
    }
}
//?}
