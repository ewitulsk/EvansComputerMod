package com.example.evanscomputermod.radio.cellular;

//? if <=1.21.1 {
import com.example.evanscomputermod.card.CardItem;
import com.example.evanscomputermod.card.CardKind;
import com.example.evanscomputermod.card.SimIdentity;
import com.example.evanscomputermod.card.SimSecret;
import net.minecraft.ChatFormatting;
import net.minecraft.network.chat.Component;
import net.minecraft.world.item.ItemStack;

import java.util.function.Consumer;

/**
 * A SIM card: a smart card ({@link com.example.evanscomputermod.card.SmartSim} is its API).
 * Blank until written, by a Card Reader/Writer on behalf of a computer ({@code cored sim new},
 * {@code simtool write}). A written card carries two components: {@code evanscomputermod:sim_card}
 * (carrier and IMSI, synced, shown here) and {@code evanscomputermod:sim_secret} (the
 * key, saved but never sent to a client). Right-click an installed LTE / mmWave modem's
 * bay with it to put it in (any SIM already there comes back out).
 */
public class SimCardItem extends CardItem {

    public SimCardItem(Properties properties) {
        super(CardKind.SIM, properties);
    }

    /** The public identity of a written card, or null for a blank card (works on clients too). */
    public static SimIdentity identity(ItemStack stack) {
        return stack.isEmpty() || !(stack.getItem() instanceof SimCardItem) ? null : stack.get(CellularContent.SIM_CARD_DATA.get());
    }

    /** The card's secret, or null. On a client it is {@link SimSecret#HIDDEN}. */
    public static SimSecret secret(ItemStack stack) {
        return stack.isEmpty() ? null : stack.get(CellularContent.SIM_SECRET.get());
    }

    /**
     * The server-side view of a written card (carrier, IMSI, key), or null for a blank card
     * or where the key isn't known (on a client).
     */
    public static SimCard sim(ItemStack stack) {
        SimIdentity id = identity(stack);
        SimSecret s = secret(stack);
        if (id == null || s == null || !s.known()) return null;
        return new SimCard(id.carrier(), id.imsi(), s.keyHex(), id.msisdn());
    }

    /** The smart card in {@code stack} (changes the stack in place), or null when it isn't a SIM card. */
    public static com.example.evanscomputermod.card.SmartSim card(ItemStack stack) {
        if (stack == null || stack.isEmpty() || !(stack.getItem() instanceof SimCardItem)) return null;
        return new com.example.evanscomputermod.card.SmartSim(new com.example.evanscomputermod.card.SmartSim.Storage() {
            @Override
            public SimIdentity identity() {
                return SimCardItem.identity(stack);
            }

            @Override
            public SimSecret secret() {
                return SimCardItem.secret(stack);
            }

            @Override
            public int count() {
                return stack.getCount();
            }

            @Override
            public void store(SimIdentity identity, SimSecret secret) {
                stack.set(CellularContent.SIM_CARD_DATA.get(), identity);
                stack.set(CellularContent.SIM_SECRET.get(), secret);
            }
        });
    }

    /** Written (has an identity). */
    public static boolean isIssued(ItemStack stack) {
        return identity(stack) != null;
    }

    @Override
    protected void addTooltip(ItemStack stack, Consumer<Component> lines) {
        super.addTooltip(stack, lines);
        SimIdentity s = identity(stack);
        if (s == null) {
            lines.accept(Component.translatable("item.evanscomputermod.sim_card.blank").withStyle(ChatFormatting.GRAY));
            return;
        }
        lines.accept(Component.translatable("item.evanscomputermod.sim_card.carrier", s.carrier()).withStyle(ChatFormatting.AQUA));
        if (!s.msisdn().isEmpty())
            lines.accept(Component.translatable("item.evanscomputermod.sim_card.number", s.numberText()).withStyle(ChatFormatting.WHITE));
        lines.accept(Component.translatable("item.evanscomputermod.sim_card.imsi", s.imsiText()).withStyle(ChatFormatting.GRAY));
        lines.accept(Component.translatable("item.evanscomputermod.sim_card.use").withStyle(ChatFormatting.DARK_GRAY));
    }
}
//?}
