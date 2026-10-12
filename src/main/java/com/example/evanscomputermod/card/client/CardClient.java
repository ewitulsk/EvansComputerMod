package com.example.evanscomputermod.card.client;

//? if <=1.21.1 {
import com.example.evanscomputermod.card.CardContent;
import net.neoforged.bus.api.IEventBus;
import net.neoforged.neoforge.client.event.RegisterMenuScreensEvent;

/** Client hookup for the Card Reader/Writer screen (loaded only on the client dist). */
public final class CardClient {
    private CardClient() {}

    public static void register(IEventBus modBus) {
        modBus.addListener((RegisterMenuScreensEvent e) -> e.register(CardContent.CARD_READER_MENU.get(), CardReaderScreen::new));
    }
}
//?}
