package com.example.evanscomputermod.card;

//? if <=1.21.1 {
import com.example.evanscomputermod.card.applet.BankSecret;
import com.example.evanscomputermod.card.applet.NfcSecret;
import com.example.evanscomputermod.phone.PhoneContent;
import com.example.evanscomputermod.phone.PhoneHardware;
import com.example.evanscomputermod.radio.cellular.CellularContent;
import com.example.evanscomputermod.radio.cellular.SimCardItem;
import net.minecraft.server.level.ServerPlayer;
import net.minecraft.world.item.ItemStack;

import java.util.LinkedHashMap;
import java.util.Map;
import java.util.UUID;
import java.util.concurrent.ConcurrentHashMap;

/**
 * Clients only ever hold hidden placeholders in place of a card's secrets ({@link SimSecret#HIDDEN},
 * {@link BankSecret#HIDDEN}, {@link NfcSecret#HIDDEN}). Survival inventories are authoritative on
 * the server, but the creative inventory sends whole stacks back
 * ({@code ServerboundSetCreativeModeSlotPacket}), which would replace a card's secrets with the
 * placeholders. Before the server applies such a stack, this puts them back: it looks for a card
 * with the same identity (a SIM's carrier + IMSI; any other card's UID) in the player's inventory,
 * the stack being replaced, or one this player held recently (moving a card is "empty the old
 * slot, fill the new one"). A card inside an ePhone's SIM tray is handled the same way. Applied by
 * {@code card.mixin.CreativeSlotMixin}.
 *
 * <p>A hidden card with no match (it can't be produced honestly) is stored without secrets: a
 * written card that answers nothing.
 */
public final class CardSecretGuard {
    private static final int REMEMBER = 64;

    /** Everything secret on one card. */
    private record Secrets(SimSecret sim, BankSecret bank, NfcSecret nfc) {}

    private static final Map<UUID, Map<String, Secrets>> RECENT = new ConcurrentHashMap<>();

    private CardSecretGuard() {}

    /** Called before the server applies a creative-inventory stack to {@code slot} (-1: dropped). */
    public static void beforeCreativeSet(ServerPlayer player, int slot, ItemStack incoming) {
        if (incoming == null || incoming.isEmpty()) {
            if (slot >= 0 && slot < player.inventoryMenu.slots.size()) remember(player, player.inventoryMenu.getSlot(slot).getItem());
            return;
        }
        if (!needsRestore(incoming)) return;
        if (slot >= 0 && slot < player.inventoryMenu.slots.size()) remember(player, player.inventoryMenu.getSlot(slot).getItem());
        var inv = player.getInventory();
        for (int i = 0; i < inv.getContainerSize(); i++) remember(player, inv.getItem(i));
        remember(player, player.containerMenu.getCarried());
        restore(player, incoming);
    }

    private static Map<String, Secrets> recent(ServerPlayer p) {
        return RECENT.computeIfAbsent(p.getUUID(), k -> java.util.Collections.synchronizedMap(new LinkedHashMap<>(16, 0.75f, true) {
            @Override
            protected boolean removeEldestEntry(Map.Entry<String, Secrets> e) {
                return size() > REMEMBER;
            }
        }));
    }

    /** What identifies a card: a SIM by carrier + IMSI, anything else by its UID; null for a card with neither. */
    private static String idKey(ItemStack card) {
        SimIdentity id = SimCardItem.identity(card);
        if (id != null) return "sim|" + id.carrier() + "|" + id.imsi();
        if (CardItem.hasUid(card)) return "uid|" + CardItem.uid(card);
        return null;
    }

    private static ItemStack cardOf(ItemStack stack) {
        if (stack.getItem() instanceof CardItem) return stack;
        PhoneHardware h = stack.get(PhoneContent.HARDWARE.get());
        return h != null ? h.sim() : ItemStack.EMPTY;
    }

    private static boolean hidden(ItemStack card) {
        SimSecret s = SimCardItem.secret(card);
        if (s != null && !s.known()) return true;
        BankSecret b = card.get(CardContent.BANK_SECRET.get());
        if (b != null && !b.known()) return true;
        NfcSecret n = card.get(CardContent.NFC_SECRET.get());
        return n != null && !n.known();
    }

    private static boolean needsRestore(ItemStack stack) {
        ItemStack card = cardOf(stack);
        if (card.isEmpty() || idKey(card) == null) return false;
        if (SimCardItem.identity(card) != null && SimCardItem.secret(card) == null) return true;   // a SIM written without a key
        return hidden(card);
    }

    /** Remember the secrets of a card (or a phone's SIM) the player has now. */
    public static void remember(ServerPlayer player, ItemStack stack) {
        if (stack == null || stack.isEmpty()) return;
        ItemStack card = cardOf(stack);
        String key = card.isEmpty() ? null : idKey(card);
        if (key == null || hidden(card)) return;
        SimSecret s = SimCardItem.secret(card);
        BankSecret b = card.get(CardContent.BANK_SECRET.get());
        NfcSecret n = card.get(CardContent.NFC_SECRET.get());
        if (s != null || b != null || n != null) recent(player).put(key, new Secrets(s, b, n));
    }

    private static void apply(ItemStack card, Secrets known) {
        if (known != null && known.sim() != null) card.set(CellularContent.SIM_SECRET.get(), known.sim());
        else card.remove(CellularContent.SIM_SECRET.get());
        if (known != null && known.bank() != null) card.set(CardContent.BANK_SECRET.get(), known.bank());
        else card.remove(CardContent.BANK_SECRET.get());
        if (known != null && known.nfc() != null) card.set(CardContent.NFC_SECRET.get(), known.nfc());
        else card.remove(CardContent.NFC_SECRET.get());
    }

    private static void restore(ServerPlayer player, ItemStack incoming) {
        ItemStack card = cardOf(incoming);
        Secrets known = recent(player).get(idKey(card));
        if (card == incoming) {
            apply(incoming, known);
            return;
        }
        ItemStack fixed = card.copy();
        apply(fixed, known);
        incoming.set(PhoneContent.HARDWARE.get(), incoming.get(PhoneContent.HARDWARE.get()).withSim(fixed));
    }
}
//?}
