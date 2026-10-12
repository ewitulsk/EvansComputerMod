package com.example.evanscomputermod.radio.cellular;

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
 * The cell tower screen's menu (no slots). The server copy pushes a fresh
 * {@link CellPackets.TowerView} every {@value #REFRESH_TICKS} ticks while it is
 * open, so attached devices, signal and data use stay live.
 */
public class CellTowerMenu extends AbstractContainerMenu {
    public static final int REFRESH_TICKS = 10;

    private final @Nullable CellTowerBlockEntity be;
    private final Player player;
    private CellPackets.TowerView view;
    private int ticks;

    /** Client. */
    public CellTowerMenu(int id, Inventory inv, RegistryFriendlyByteBuf buf) {
        super(CellularContent.CELL_TOWER_MENU.get(), id);
        this.be = null;
        this.player = inv.player;
        this.view = CellPackets.readView(buf);
    }

    /** Server. */
    public CellTowerMenu(int id, Inventory inv, CellTowerBlockEntity be) {
        super(CellularContent.CELL_TOWER_MENU.get(), id);
        this.be = be;
        this.player = inv.player;
        this.view = CellPackets.view(be, null);
    }

    public CellPackets.TowerView view() {
        return view;
    }

    public void setView(CellPackets.TowerView v) {
        view = v;
    }

    @Override
    public void broadcastChanges() {
        super.broadcastChanges();
        if (be != null && player instanceof ServerPlayer sp && ++ticks % REFRESH_TICKS == 0) {
            view = CellPackets.view(be, null);
            PacketDistributor.sendToPlayer(sp, new CellPackets.Status(view));
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
                && p.distanceToSqr(be.getBlockPos().getCenter()) <= CellPackets.MAX_DISTANCE * CellPackets.MAX_DISTANCE;
    }
}
//?}
