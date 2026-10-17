package com.example.evanscomputermod.card.nfc;

//? if <=1.21.1 {
import com.example.evanscomputermod.card.CardContent;
import com.example.evanscomputermod.radio.api.Pose;
import net.minecraft.core.BlockPos;
import net.minecraft.core.HolderLookup;
import net.minecraft.nbt.CompoundTag;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.block.entity.BlockEntity;
import net.minecraft.world.level.block.state.BlockState;

import java.util.UUID;

/**
 * The NFC Reader's state: its {@link NfcRadio} (the coil, at the pad's centre) and the
 * peripheral. The field setting is saved so a powered reader comes back energised.
 */
public class NfcReaderBlockEntity extends BlockEntity {
    public static final double MAX_DBM = 10;

    private UUID id = UUID.randomUUID();
    private NfcRadio radio;
    private NfcReaderPeripheral peripheral;
    private boolean fieldSaved;
    private boolean loaded;

    public NfcReaderBlockEntity(BlockPos pos, BlockState state) {
        super(CardContent.NFC_READER_BE.get(), pos, state);
    }

    private NfcRadio radio() {
        if (radio == null) {
            radio = new NfcRadio(id, this::pose, com.example.evanscomputermod.radio.medium.RadioMediumHooks::medium, MAX_DBM);
            peripheral = new NfcReaderPeripheral(radio, this);
        }
        return radio;
    }

    public NfcReaderPeripheral peripheral() {
        radio();
        return peripheral;
    }

    private Pose pose() {
        String dim = level == null ? "minecraft:overworld" : level.dimension().location().toString();
        return Pose.at(dim, worldPosition.getX() + 0.5, worldPosition.getY() + 0.2, worldPosition.getZ() + 0.5);
    }

    public static void serverTick(Level level, BlockPos pos, BlockState state, NfcReaderBlockEntity be) {
        NfcRadio r = be.radio();
        if (!be.loaded) {
            be.loaded = true;
            r.load();
            if (be.fieldSaved) r.setField(true);
        }
        r.tick();
        be.peripheral.tick(level.getGameTime());
        if (state.hasProperty(NfcReaderBlock.FIELD) && state.getValue(NfcReaderBlock.FIELD) != r.field())
            level.setBlock(pos, state.setValue(NfcReaderBlock.FIELD, r.field()), 3);
        if (r.field() != be.fieldSaved) {
            be.fieldSaved = r.field();
            be.setChanged();
        }
    }

    @Override
    public void setRemoved() {
        super.setRemoved();
        if (radio != null) radio.unload();
        loaded = false;
    }

    @Override
    public void onChunkUnloaded() {
        super.onChunkUnloaded();
        if (radio != null) radio.unload();
        loaded = false;
    }

    @Override
    protected void saveAdditional(CompoundTag tag, HolderLookup.Provider registries) {
        super.saveAdditional(tag, registries);
        tag.putUUID("Id", id);
        tag.putBoolean("Field", radio != null ? radio.field() : fieldSaved);
        if (radio != null) tag.putDouble("Power", radio.txDbm());
    }

    @Override
    protected void loadAdditional(CompoundTag tag, HolderLookup.Provider registries) {
        super.loadAdditional(tag, registries);
        if (tag.hasUUID("Id")) id = tag.getUUID("Id");
        fieldSaved = tag.getBoolean("Field");
        if (tag.contains("Power")) radio().setTxDbm(tag.getDouble("Power"));
    }
}
//?}
