package com.example.evanscomputermod.card;

//? if <=1.21.1 {
import com.example.evanscomputermod.radio.amp.AmplifierBlockEntity;
import net.minecraft.core.BlockPos;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.server.level.ServerPlayer;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.level.block.entity.BlockEntity;
import net.neoforged.bus.api.SubscribeEvent;
import net.neoforged.neoforge.event.tick.PlayerTickEvent;

import java.util.Map;
import java.util.Random;
import java.util.UUID;
import java.util.concurrent.ConcurrentHashMap;

/**
 * Strong fields scramble magnetic stripes: a player who stands within {@link #RADIUS}
 * blocks of a transmitting amplifier for {@link #SECONDS}
 * seconds has every stripe in their inventory corrupted (random characters, not blanks:
 * programs that trust a stripe see garbage and must validate it). Checked once a second.
 */
public final class StripeHazard {
    public static final double RADIUS = 2.5;
    public static final int SECONDS = 3;
    private static final Map<UUID, Integer> EXPOSURE = new ConcurrentHashMap<>();
    private static final Random RND = new Random();

    private StripeHazard() {}

    @SubscribeEvent
    public static void onPlayerTick(PlayerTickEvent.Post e) {
        if (!(e.getEntity() instanceof ServerPlayer p) || p.tickCount % 20 != 0) return;
        boolean near = nearField(p.serverLevel(), p.blockPosition());
        int s = near ? EXPOSURE.merge(p.getUUID(), 1, Integer::sum) : 0;
        if (!near) EXPOSURE.remove(p.getUUID());
        if (s >= SECONDS) {
            EXPOSURE.put(p.getUUID(), 0);
            scramble(p);
        }
    }

    static boolean nearField(ServerLevel level, BlockPos at) {
        int r = (int) Math.ceil(RADIUS);
        for (BlockPos pos : BlockPos.betweenClosed(at.offset(-r, -r, -r), at.offset(r, r, r))) {
            if (!pos.closerThan(at, RADIUS)) continue;
            BlockEntity be = level.getBlockEntity(pos);
            if (be instanceof AmplifierBlockEntity a && a.transmitting()) return true;
        }
        return false;
    }

    /** Corrupt every stripe the player carries; true if any changed. */
    public static boolean scramble(ServerPlayer p) {
        boolean any = false;
        var inv = p.getInventory();
        for (int i = 0; i < inv.getContainerSize(); i++) {
            ItemStack s = inv.getItem(i);
            Stripe st = CardItem.stripe(s);
            if (st == null || st.isEmpty() || st.corrupted()) continue;
            s.set(CardContent.CARD_STRIPE.get(), st.corrupt(RND));
            any = true;
        }
        if (any) p.displayClientMessage(net.minecraft.network.chat.Component.translatable("message.evanscomputermod.card.stripe_scrambled"), true);
        return any;
    }
}
//?}
