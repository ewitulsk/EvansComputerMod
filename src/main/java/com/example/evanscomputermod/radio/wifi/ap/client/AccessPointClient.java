package com.example.evanscomputermod.radio.wifi.ap.client;

//? if <=1.21.1 {
import com.example.evanscomputermod.radio.wifi.ap.AccessPointContent;
import com.example.evanscomputermod.radio.wifi.ap.AccessPointMenu;
import com.example.evanscomputermod.radio.wifi.ap.ApPackets;
import net.minecraft.client.Minecraft;
import net.neoforged.bus.api.IEventBus;
import net.neoforged.neoforge.client.event.RegisterMenuScreensEvent;

/** Client hookup for the Access Point screen (loaded only on the client dist). */
public final class AccessPointClient {
    private AccessPointClient() {}

    public static void register(IEventBus modBus) {
        modBus.addListener((RegisterMenuScreensEvent e) -> e.register(AccessPointContent.ACCESS_POINT_MENU.get(), AccessPointScreen::new));
    }

    /** A status refresh: update the open AP menu if it is for this block. */
    public static void onStatus(ApPackets.ApView view) {
        var player = Minecraft.getInstance().player;
        if (player != null && player.containerMenu instanceof AccessPointMenu m && m.view().pos().equals(view.pos())) m.setView(view);
    }
}
//?}
