package com.example.evanscomputermod.radio.cellular.client;

//? if <=1.21.1 {
import com.example.evanscomputermod.radio.cellular.CellPackets;
import com.example.evanscomputermod.radio.cellular.CellTowerMenu;
import com.example.evanscomputermod.radio.cellular.CellularContent;
import net.minecraft.client.Minecraft;
import net.neoforged.bus.api.IEventBus;
import net.neoforged.neoforge.client.event.RegisterMenuScreensEvent;

/** Client hookup for the cell tower screen (loaded only on the client dist). */
public final class CellClient {
    private CellClient() {}

    public static void register(IEventBus modBus) {
        modBus.addListener((RegisterMenuScreensEvent e) -> e.register(CellularContent.CELL_TOWER_MENU.get(), CellTowerScreen::new));
    }

    /** A status refresh: update the open tower menu if it is for this block. */
    public static void onStatus(CellPackets.TowerView view) {
        var player = Minecraft.getInstance().player;
        if (player != null && player.containerMenu instanceof CellTowerMenu m && m.view().pos().equals(view.pos())) m.setView(view);
    }
}
//?}
