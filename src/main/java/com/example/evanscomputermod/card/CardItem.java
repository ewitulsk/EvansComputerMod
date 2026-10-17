package com.example.evanscomputermod.card;

//? if <=1.21.1 {
import com.example.evanscomputermod.api.module.IBayUseItem;
import com.example.evanscomputermod.card.applet.BankApplet;
import com.example.evanscomputermod.card.applet.BankPublic;
import com.example.evanscomputermod.card.applet.BankSecret;
import com.example.evanscomputermod.card.applet.NfcApplet;
import com.example.evanscomputermod.card.applet.NfcPublic;
import com.example.evanscomputermod.card.applet.NfcSecret;
import com.example.evanscomputermod.item.TooltipItem;
import net.minecraft.ChatFormatting;
import net.minecraft.network.chat.Component;
import net.minecraft.world.item.ItemStack;
import org.jetbrains.annotations.Nullable;

import java.util.Map;
import java.util.function.Consumer;

/**
 * A card (docs/cards/CARDS.md): one chassis per {@link CardKind}, carrying painted faces
 * ({@link CardFaces}), printed fields, a magnetic stripe (bank, hybrid) and one or more
 * applets: the bank chip ({@link BankApplet}), the NFC sectors ({@link NfcApplet}) and,
 * on a SIM, the SIM ({@link SmartSim}, see {@link com.example.evanscomputermod.radio.cellular.SimCardItem}).
 * Every applet's public half is a synced component; the secret halves never reach a client.
 * The static helpers here are the storage adapters the applets run over.
 */
public class CardItem extends TooltipItem implements IBayUseItem {
    private final CardKind kind;

    public CardItem(CardKind kind, Properties properties) {
        super(properties);
        this.kind = kind;
    }

    public CardKind kind() {
        return kind;
    }

    /** The kind of a card stack, or null for anything else. */
    public static @Nullable CardKind kindOf(ItemStack stack) {
        return stack != null && !stack.isEmpty() && stack.getItem() instanceof CardItem c ? c.kind : null;
    }

    public static boolean isCard(ItemStack stack) {
        return kindOf(stack) != null || (stack != null && !stack.isEmpty() && stack.is(CardContent.CARDS));
    }

    // ------------------------------------------------------------ public components

    public static @Nullable CardFaces faces(ItemStack stack) {
        return stack.get(CardContent.CARD_FACES.get());
    }

    public static Map<String, String> fields(ItemStack stack) {
        Map<String, String> m = stack.get(CardContent.CARD_PRINT.get());
        return m == null ? Map.of() : m;
    }

    public static @Nullable Stripe stripe(ItemStack stack) {
        CardKind k = kindOf(stack);
        if (k == null || !k.stripe) return null;
        Stripe s = stack.get(CardContent.CARD_STRIPE.get());
        return s == null ? Stripe.EMPTY : s;
    }

    /** The card's UID, or 0 before any applet was written. */
    public static long uid(ItemStack stack) {
        Long u = stack.get(CardContent.CARD_UID.get());
        return u == null ? 0 : u;
    }

    public static boolean hasUid(ItemStack stack) {
        return stack.has(CardContent.CARD_UID.get());
    }

    static void ensureUid(ItemStack stack) {
        if (!stack.has(CardContent.CARD_UID.get())) {
            long u;
            do {
                u = new java.security.SecureRandom().nextLong();
            } while (u == 0);
            stack.set(CardContent.CARD_UID.get(), u);
        }
    }

    // ------------------------------------------------------------ applets

    /** The bank chip of a bank / hybrid card (changes the stack in place), or null. */
    public static @Nullable BankApplet bank(ItemStack stack) {
        CardKind k = kindOf(stack);
        if (k == null || !k.bank) return null;
        return new BankApplet(new BankApplet.Storage() {
            @Override
            public long uid() {
                return CardItem.uid(stack);
            }

            @Override
            public void ensureUid() {
                CardItem.ensureUid(stack);
            }

            @Override
            public BankPublic bankPublic() {
                return stack.get(CardContent.BANK_APPLET.get());
            }

            @Override
            public BankSecret bankSecret() {
                return stack.get(CardContent.BANK_SECRET.get());
            }

            @Override
            public int count() {
                return stack.getCount();
            }

            @Override
            public void store(BankPublic pub, BankSecret secret) {
                stack.set(CardContent.BANK_APPLET.get(), pub);
                if (secret != null) stack.set(CardContent.BANK_SECRET.get(), secret);
            }
        });
    }

    /** The NFC sectors of an nfc / hybrid card (changes the stack in place), or null. */
    public static @Nullable NfcApplet nfc(ItemStack stack) {
        CardKind k = kindOf(stack);
        if (k == null || !k.nfc) return null;
        return new NfcApplet(new NfcApplet.Storage() {
            @Override
            public long uid() {
                return CardItem.uid(stack);
            }

            @Override
            public void ensureUid() {
                CardItem.ensureUid(stack);
            }

            @Override
            public NfcPublic nfcPublic() {
                return stack.get(CardContent.NFC_APPLET.get());
            }

            @Override
            public NfcSecret nfcSecret() {
                return stack.get(CardContent.NFC_SECRET.get());
            }

            @Override
            public void store(NfcPublic pub, NfcSecret secret) {
                stack.set(CardContent.NFC_APPLET.get(), pub);
                stack.set(CardContent.NFC_SECRET.get(), secret);
            }
        });
    }

    /** The SIM of a SIM card (see {@code SimCardItem.card}), or null. */
    public static @Nullable SmartSim simOf(ItemStack stack) {
        return com.example.evanscomputermod.radio.cellular.SimCardItem.card(stack);
    }

    /** The applet names a card carries: what {@code identity().applets} lists. */
    public static java.util.List<String> applets(ItemStack stack) {
        CardKind k = kindOf(stack);
        if (k == null) return java.util.List.of();
        java.util.List<String> l = new java.util.ArrayList<>(3);
        if (k.bank) l.add("bank");
        if (k.nfc) l.add("nfc");
        if (k.sim) l.add("sim");
        return l;
    }

    /** Every applet of the card is blank (unwritten): a fresh card, whatever is printed on it. */
    public static boolean allBlank(ItemStack stack) {
        CardKind k = kindOf(stack);
        if (k == null) return false;
        if (k.bank && stack.has(CardContent.BANK_APPLET.get())) return false;
        if (k.nfc && stack.has(CardContent.NFC_APPLET.get())) return false;
        if (k.sim && simOf(stack) != null && !simOf(stack).isBlank()) return false;
        return true;
    }

    @Override
    public boolean isFoil(ItemStack stack) {
        return false;
    }

    @Override
    protected void addTooltip(ItemStack stack, Consumer<Component> lines) {
        Map<String, String> f = fields(stack);
        if (f.containsKey("name")) lines.accept(Component.literal(f.get("name")).withStyle(ChatFormatting.WHITE));
        if (f.containsKey("issuer")) lines.accept(Component.literal(f.get("issuer")).withStyle(ChatFormatting.AQUA));
        BankPublic b = stack.get(CardContent.BANK_APPLET.get());
        if (kind.bank) {
            if (b == null) lines.accept(Component.translatable("item.evanscomputermod.card.bank_blank").withStyle(ChatFormatting.GRAY));
            else {
                lines.accept(Component.translatable("item.evanscomputermod.card.bank", b.issuer(), b.panText()).withStyle(ChatFormatting.GRAY));
                if (b.locked()) lines.accept(Component.translatable("item.evanscomputermod.card.locked").withStyle(ChatFormatting.RED));
            }
            Stripe s = stripe(stack);
            if (s != null && s.corrupted()) lines.accept(Component.translatable("item.evanscomputermod.card.stripe_corrupted").withStyle(ChatFormatting.RED));
        }
        if (kind.nfc) {
            NfcPublic n = stack.get(CardContent.NFC_APPLET.get());
            lines.accept(Component.translatable(n == null || n.writtenCount() == 0 ? "item.evanscomputermod.card.nfc_factory"
                    : "item.evanscomputermod.card.nfc", n == null ? 0 : n.writtenCount()).withStyle(ChatFormatting.GRAY));
        }
        if (hasUid(stack)) lines.accept(Component.translatable("item.evanscomputermod.card.uid", Hex.ofLong(uid(stack))).withStyle(ChatFormatting.DARK_GRAY));
        if (faces(stack) == null) lines.accept(Component.translatable("item.evanscomputermod.card.unprinted").withStyle(ChatFormatting.DARK_GRAY));
    }
}
//?}
