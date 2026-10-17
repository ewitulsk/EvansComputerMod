package com.example.evanscomputermod.card.nfc;

//? if <=1.21.1 {
import com.example.evanscomputermod.api.module.IComputerModule;
import com.example.evanscomputermod.api.module.IModuleHost;
import com.example.evanscomputermod.api.peripheral.AnnotatedPeripheral;
import com.example.evanscomputermod.api.peripheral.IComputerAccess;
import com.example.evanscomputermod.api.peripheral.IPeripheral;
import com.example.evanscomputermod.api.peripheral.PeripheralException;
import net.minecraft.nbt.CompoundTag;

import java.util.Set;
import java.util.UUID;

/** The live NFC module: an {@link NfcRadio} whose coil follows the host (a block, or a phone in a hand). */
public final class NfcModule extends AnnotatedPeripheral implements IComputerModule {
    public static final double MAX_DBM = 6;

    private final IModuleHost host;
    private final NfcRadio radio;
    private final NfcReaderPeripheral peripheral;
    private long ticks;

    NfcModule(IModuleHost host, CompoundTag saved) {
        this.host = host;
        UUID id = saved.hasUUID("Id") ? saved.getUUID("Id") : UUID.randomUUID();
        radio = new NfcRadio(id, host::getPose, com.example.evanscomputermod.radio.medium.RadioMediumHooks::medium, MAX_DBM);
        peripheral = new NfcReaderPeripheral(radio, this);
        if (saved.contains("Power")) radio.setTxDbm(saved.getDouble("Power"));
        if (saved.getBoolean("Field")) radio.setField(true);
    }

    @Override
    public String getType() {
        return NfcReaderPeripheral.TYPE;
    }

    @Override
    public Set<String> getMethodNames() {
        return peripheral.getMethodNames();
    }

    @Override
    public Object callMethod(IComputerAccess computer, String method, Object[] arguments) throws PeripheralException {
        Object r = peripheral.callMethod(computer, method, arguments);
        host.markDirty();
        return r;
    }

    @Override
    public boolean runsOnMainThread(String method) {
        return peripheral.runsOnMainThread(method);
    }

    @Override
    public void attach(IComputerAccess computer) {
        peripheral.attach(computer);
    }

    @Override
    public void detach(IComputerAccess computer) {
        peripheral.detach(computer);
    }

    @Override
    public boolean isSame(IPeripheral other) {
        return other == this;
    }

    @Override
    public void onLoad() {
        radio.load();
    }

    @Override
    public void onUnload() {
        radio.unload();
    }

    @Override
    public void tick() {
        radio.tick();
        peripheral.tick(++ticks);
    }

    @Override
    public void saveState(CompoundTag tag) {
        tag.putUUID("Id", radio.id());
        tag.putBoolean("Field", radio.field());
        tag.putDouble("Power", radio.txDbm());
    }
}
//?}
