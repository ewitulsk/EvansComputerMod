package com.example.evanscomputermod.radio.wifi.ap;

//? if <=1.21.1 {
import net.minecraft.network.RegistryFriendlyByteBuf;
import net.minecraft.server.level.ServerPlayer;
import net.minecraft.world.entity.player.Inventory;
import net.minecraft.world.entity.player.Player;
import net.minecraft.world.inventory.AbstractContainerMenu;
import net.minecraft.world.item.ItemStack;
import net.neoforged.neoforge.network.PacketDistributor;
import org.jetbrains.annotations.Nullable;

/**
 * The Access Point screen's menu (no slots). The client copy holds the
 * latest {@link ApPackets.ApView}; the server copy pushes a fresh one every
 * {@value #REFRESH_TICKS} ticks while the screen is open, so the status page
 * (clients, RSSI, rate, handshake state) stays live.
 */
public class AccessPointMenu extends AbstractContainerMenu {
    public static final int REFRESH_TICKS = 10;

    private final @Nullable AccessPointBlockEntity be;
    private final Player player;
    private ApPackets.ApView view;
    private int ticks;

    /** Client. */
    public AccessPointMenu(int id, Inventory inv, RegistryFriendlyByteBuf buf) {
        super(AccessPointContent.ACCESS_POINT_MENU.get(), id);
        this.be = null;
        this.player = inv.player;
        this.view = ApPackets.readView(buf);
    }

    /** Server. */
    public AccessPointMenu(int id, Inventory inv, AccessPointBlockEntity be) {
        super(AccessPointContent.ACCESS_POINT_MENU.get(), id);
        this.be = be;
        this.player = inv.player;
        this.view = be.view(null);
    }

    public ApPackets.ApView view() {
        return view;
    }

    public void setView(ApPackets.ApView v) {
        view = v;
    }

    @Override
    public void broadcastChanges() {
        super.broadcastChanges();
        if (be != null && player instanceof ServerPlayer sp && ++ticks % REFRESH_TICKS == 0) {
            view = be.view(null);
            PacketDistributor.sendToPlayer(sp, new ApPackets.Status(view));
        }
    }

    @Override
    public ItemStack quickMoveStack(Player player, int index) {
        return ItemStack.EMPTY;
    }

    @Override
    public boolean stillValid(Player p) {
        if (be == null) return true;
        return !be.isRemoved() && be.canConfigure(p)
                && p.distanceToSqr(be.getBlockPos().getCenter()) <= ApPackets.MAX_DISTANCE * ApPackets.MAX_DISTANCE;
    }
}
//?}
