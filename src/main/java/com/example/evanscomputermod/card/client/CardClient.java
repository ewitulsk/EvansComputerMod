package com.example.evanscomputermod.card.client;

//? if <=1.21.1 {
import com.example.evanscomputermod.card.CardContent;
import com.example.evanscomputermod.card.CardKind;
import com.example.evanscomputermod.radio.cellular.CellularContent;
import net.minecraft.client.renderer.BlockEntityWithoutLevelRenderer;
import net.minecraft.world.item.Item;
import net.neoforged.bus.api.IEventBus;
import net.neoforged.neoforge.client.event.ModelEvent;
import net.neoforged.neoforge.client.event.RegisterMenuScreensEvent;
import net.neoforged.neoforge.client.event.RenderFrameEvent;
import net.neoforged.neoforge.client.extensions.common.IClientItemExtensions;
import net.neoforged.neoforge.client.extensions.common.RegisterClientExtensionsEvent;
import net.neoforged.neoforge.common.NeoForge;

/** Client hookup for cards: the item renderer, the body models and the screens. */
public final class CardClient {
    private CardClient() {}

    public static void register(IEventBus modBus) {
        modBus.addListener((RegisterMenuScreensEvent e) -> {
            e.register(CardContent.CARD_READER_MENU.get(), CardReaderScreen::new);
            e.register(CardContent.CARD_PRINTER_MENU.get(), CardPrinterScreen::new);
        });
        modBus.addListener((RegisterClientExtensionsEvent e) -> e.registerItem(new IClientItemExtensions() {
            private CardItemRenderer renderer;

            @Override
            public BlockEntityWithoutLevelRenderer getCustomRenderer() {
                if (renderer == null) renderer = new CardItemRenderer();
                return renderer;
            }
        }, cardItems()));
        modBus.addListener((ModelEvent.RegisterAdditional e) -> {
            for (CardKind k : CardKind.values()) e.register(CardItemRenderer.BODIES.get(k));
        });
        NeoForge.EVENT_BUS.addListener((RenderFrameEvent.Post e) -> CardFaceTexture.endFrame());
    }

    private static Item[] cardItems() {
        return new Item[] {CardContent.BANK_CARD.get(), CardContent.NFC_CARD.get(), CardContent.HYBRID_CARD.get(), CellularContent.SIM_CARD.get()};
    }
}
//?}
