package com.example.evanscomputermod.card;

//? if <=1.21.1 {
import com.example.evanscomputermod.api.module.IComputerModule;
import com.example.evanscomputermod.api.module.IModuleHost;
import com.example.evanscomputermod.api.module.IModuleItemTarget;
import com.example.evanscomputermod.api.peripheral.AnnotatedPeripheral;
import com.example.evanscomputermod.api.peripheral.IComputerAccess;
import com.example.evanscomputermod.api.peripheral.IPeripheral;
import com.example.evanscomputermod.api.peripheral.PeripheralException;
import com.example.evanscomputermod.radio.cellular.ModemSimStore;
import net.minecraft.nbt.CompoundTag;
import net.minecraft.network.chat.Component;
import net.minecraft.world.entity.player.Player;
import net.minecraft.world.item.ItemStack;

import java.util.Map;
import java.util.Set;

/**
 * The live card reader module: one card slot in a bay. The slot's card is the server-side
 * stack in {@link ModemSimStore} (its slot id in the module state); the module itself is
 * the {@link CardHolder} a {@link CardReaderPeripheral} reads. "Release" with a module just
 * means the computer is done: there is nothing to push the card into, so Auto Eject is
 * always false and the card waits to be taken (sneak-right-click the bay).
 */
public final class CardReaderModule extends AnnotatedPeripheral implements IComputerModule, IModuleItemTarget, CardHolder {
    private final IModuleHost host;
    private volatile String slot;
    private volatile ItemStack card = ItemStack.EMPTY;
    private boolean released;
    private final CardReaderPeripheral reader = new CardReaderPeripheral(this);

    CardReaderModule(IModuleHost host, CompoundTag saved) {
        this.host = host;
        this.slot = ModemSimStore.slotOf(saved.getCompound("card"));
        this.card = ModemSimStore.peek(slot);
        if (card.isEmpty()) slot = null;
    }

    @Override
    public String getType() {
        return CardReaderPeripheral.TYPE;
    }

    @Override
    public Set<String> getMethodNames() {
        return reader.getMethodNames();
    }

    @Override
    public Object callMethod(IComputerAccess computer, String method, Object[] arguments) throws PeripheralException {
        return reader.callMethod(computer, method, arguments);
    }

    @Override
    public boolean runsOnMainThread(String method) {
        return reader.runsOnMainThread(method);
    }

    @Override
    public void attach(IComputerAccess computer) {
        reader.attach(computer);
    }

    @Override
    public void detach(IComputerAccess computer) {
        reader.detach(computer);
    }

    @Override
    public boolean isSame(IPeripheral other) {
        return other == this;
    }

    @Override
    public Map<String, IPeripheral> companions() {
        return Map.of();
    }

    // ------------------------------------------------------------ CardHolder

    @Override
    public ItemStack card() {
        return card;
    }

    @Override
    public boolean release() {
        if (card.isEmpty()) return false;
        released = true;
        return false;
    }

    @Override
    public void cardChanged() {
        host.markDirty();
    }

    @Override
    public boolean released() {
        return released;
    }

    @Override
    public boolean autoEject() {
        return false;
    }

    // ------------------------------------------------------------ the bay slot

    @Override
    public boolean acceptsItem(ItemStack stack) {
        return CardItem.isCard(stack);
    }

    @Override
    public Component useItem(ItemStack stack, Player player) {
        if (!card.isEmpty()) return Component.translatable("message.evanscomputermod.card_reader.occupied");
        String id = ModemSimStore.put(stack);
        if (id == null) return Component.translatable("message.evanscomputermod.no_module_takes_item");
        slot = id;
        card = ModemSimStore.peek(id);
        released = false;
        if (!player.getAbilities().instabuild) stack.shrink(1);
        host.markDirty();
        reader.cardInserted(card);
        return Component.translatable("message.evanscomputermod.card_reader.inserted");
    }

    /** Take the card out (the bay's sneak-use); EMPTY with none. */
    public ItemStack takeCard() {
        ItemStack out = ModemSimStore.take(slot);
        slot = null;
        card = ItemStack.EMPTY;
        released = false;
        host.markDirty();
        if (!out.isEmpty()) reader.cardRemoved();
        return out;
    }

    @Override
    public void saveState(CompoundTag tag) {
        if (slot != null && !card.isEmpty()) {
            CompoundTag c = new CompoundTag();
            c.putString("slot", slot);
            c.putString("kind", CardReaderPeripheral.kindName(card));
            tag.put("card", c);
        }
    }
}
//?}
