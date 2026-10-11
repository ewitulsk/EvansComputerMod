package com.example.evanscomputermod.energy;

//? if <=1.21.1 {
import net.minecraft.nbt.CompoundTag;
import net.neoforged.neoforge.energy.EnergyStorage;

/**
 * FE buffer for radio hardware (generators, amplifiers). A plain NeoForge
 * {@link EnergyStorage} with a save/load helper and a change callback so block
 * entities can mark themselves dirty. Written so computers can adopt it later.
 */
public class RadioEnergyStorage extends EnergyStorage {

    private final Runnable onChange;

    public RadioEnergyStorage(int capacity, int maxReceive, int maxExtract, Runnable onChange) {
        super(capacity, maxReceive, maxExtract);
        this.onChange = onChange;
    }

    @Override
    public int receiveEnergy(int amount, boolean simulate) {
        int r = super.receiveEnergy(amount, simulate);
        if (r != 0 && !simulate) onChange.run();
        return r;
    }

    @Override
    public int extractEnergy(int amount, boolean simulate) {
        int r = super.extractEnergy(amount, simulate);
        if (r != 0 && !simulate) onChange.run();
        return r;
    }

    /** Generators fill themselves regardless of maxReceive. Returns the amount stored. */
    public int generate(int amount) {
        int r = Math.min(capacity - energy, Math.max(0, amount));
        if (r != 0) {
            energy += r;
            onChange.run();
        }
        return r;
    }

    /** Machines draw from themselves regardless of maxExtract. Returns the amount drawn. */
    public int consume(int amount) {
        int r = Math.min(energy, Math.max(0, amount));
        if (r != 0) {
            energy -= r;
            onChange.run();
        }
        return r;
    }

    public void setEnergy(int value) {
        energy = Math.max(0, Math.min(capacity, value));
    }

    public void save(CompoundTag tag, String key) {
        tag.putInt(key, energy);
    }

    public void load(CompoundTag tag, String key) {
        setEnergy(tag.getInt(key));
    }
}
//?}
