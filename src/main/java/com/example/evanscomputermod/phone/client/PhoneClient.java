package com.example.evanscomputermod.phone.client;

//? if <=1.21.1 {
import com.example.evanscomputermod.network.ClientDisplayCache;
import com.example.evanscomputermod.network.TerminalReadyPacket;
import com.example.evanscomputermod.phone.PhoneClientConfig;
import com.example.evanscomputermod.phone.PhoneConfig;
import com.example.evanscomputermod.phone.PhoneContent;
import com.example.evanscomputermod.phone.PhoneItemData;
import com.example.evanscomputermod.phone.PhonePackets;
import com.example.evanscomputermod.phone.PhoneState;
import com.mojang.blaze3d.platform.InputConstants;
import net.minecraft.client.KeyMapping;
import net.minecraft.client.Minecraft;
import net.minecraft.client.player.LocalPlayer;
import net.minecraft.world.InteractionHand;
import net.minecraft.world.item.ItemStack;
import net.neoforged.bus.api.IEventBus;
import net.neoforged.neoforge.client.event.ClientPlayerNetworkEvent;
import net.neoforged.neoforge.client.event.ClientTickEvent;
import net.neoforged.neoforge.client.event.EntityRenderersEvent;
import net.neoforged.neoforge.client.event.ModelEvent;
import net.neoforged.neoforge.client.event.RegisterKeyMappingsEvent;
import net.neoforged.neoforge.client.event.RegisterMenuScreensEvent;
import net.neoforged.neoforge.client.event.RenderFrameEvent;
import net.neoforged.neoforge.client.extensions.common.IClientItemExtensions;
import net.neoforged.neoforge.client.extensions.common.RegisterClientExtensionsEvent;
import net.neoforged.neoforge.client.settings.KeyConflictContext;
import net.neoforged.neoforge.common.NeoForge;
import net.neoforged.neoforge.network.PacketDistributor;
import org.jetbrains.annotations.Nullable;

import java.util.Map;
import java.util.UUID;
import java.util.concurrent.ConcurrentHashMap;

/**
 * Client side of the ePhone: its renderers and screens, the status of phones
 * nearby, frame requests, and the in-hand hotkeys.
 *
 * <p>Hotkeys: seven key mappings in Controls, "ePhone" category, unbound by
 * default: hotkey slots 1-6 and "switch app". They act only while an ePhone is
 * in the main hand or off hand and no screen is open, and they never touch any
 * other key mapping: a slot bound to W both walks (vanilla's mapping on the same
 * key still fires; NeoForge sets every mapping bound to a key) and presses the
 * slot. Presses go to the server addressed by the phone's UUID
 * ({@link PhonePackets.Hotkey}); the phone hands them to its
 * {@code PhoneHotkeySink}.
 */
public final class PhoneClient {
    public static final String CATEGORY = "key.categories.evanscomputermod.ephone";
    public static final KeyMapping[] SLOTS = new KeyMapping[6];
    public static final KeyMapping SWITCH_APP = new KeyMapping("key.evanscomputermod.ephone.switch_app",
            KeyConflictContext.IN_GAME, InputConstants.UNKNOWN, CATEGORY);

    static {
        for (int i = 0; i < 6; i++)
            SLOTS[i] = new KeyMapping("key.evanscomputermod.ephone.slot" + (i + 1), KeyConflictContext.IN_GAME,
                    InputConstants.UNKNOWN, CATEGORY);
    }

    /** A phone's last reported status. */
    public record Status(PhoneState state, PhoneConfig.Visibility visibility, double battery, long at) {}

    private static final Map<UUID, Status> STATUS = new ConcurrentHashMap<>();
    private static final Map<String, Long> FRAME_REQUESTS = new ConcurrentHashMap<>();
    private static final boolean[] down = new boolean[6];
    @Nullable private static UUID downPhone;
    private static int hotkeysSent;
    @Nullable private static PhonePackets.Prefs sentPrefs;

    private PhoneClient() {}

    public static void register(IEventBus modBus) {
        modBus.addListener((RegisterKeyMappingsEvent e) -> {
            for (KeyMapping k : SLOTS) e.register(k);
            e.register(SWITCH_APP);
        });
        modBus.addListener((RegisterClientExtensionsEvent e) -> e.registerItem(new IClientItemExtensions() {
            private PhoneItemRenderer renderer;

            @Override
            public net.minecraft.client.renderer.BlockEntityWithoutLevelRenderer getCustomRenderer() {
                if (renderer == null) renderer = new PhoneItemRenderer();
                return renderer;
            }
        }, PhoneContent.EPHONE.get()));
        modBus.addListener((ModelEvent.RegisterAdditional e) -> e.register(PhoneItemRenderer.BODY));
        modBus.addListener((EntityRenderersEvent.RegisterRenderers e) ->
                e.registerBlockEntityRenderer(PhoneContent.CHARGING_DOCK_BE.get(), ChargingDockRenderer::new));
        modBus.addListener((RegisterMenuScreensEvent e) -> e.register(PhoneContent.PHONE_MENU.get(), PhoneMenuScreen::new));
        NeoForge.EVENT_BUS.addListener((ClientTickEvent.Post e) -> tick());
        NeoForge.EVENT_BUS.addListener((RenderFrameEvent.Post e) -> PhoneScreenTexture.endFrame());
        NeoForge.EVENT_BUS.addListener((ClientPlayerNetworkEvent.LoggingIn e) -> sentPrefs = null);
        NeoForge.EVENT_BUS.addListener((ClientPlayerNetworkEvent.LoggingOut e) -> {
            STATUS.clear();
            FRAME_REQUESTS.clear();
            PhoneScreenTexture.clearAll();
            sentPrefs = null;
        });
        if (com.example.evanscomputermod.testing.RadioVisualLayout.suite().equals("phone"))
            NeoForge.EVENT_BUS.addListener(com.example.evanscomputermod.testing.client.PhoneClientChecks::tick);
    }

    // ------------------------------------------------------------ status and frames

    public static void onStatus(PhonePackets.Status s) {
        STATUS.put(s.phone(), new Status(s.phoneState(), s.phoneVisibility(), s.batteryCenti() / 100.0, System.currentTimeMillis()));
    }

    /** The phone's status if the server reported it in the last 5 seconds. */
    @Nullable
    public static Status status(UUID id) {
        Status s = STATUS.get(id);
        return s != null && System.currentTimeMillis() - s.at() < 5_000 ? s : null;
    }

    /** Ask for a keyframe of a phone display this client has no picture for (at most once a second). */
    public static void wantFrames(UUID id, int displayId) {
        if (ClientDisplayCache.entry(id, displayId).hasKeyframe || Minecraft.getInstance().getConnection() == null) return;
        String key = id + "/" + displayId;
        long now = System.currentTimeMillis();
        Long last = FRAME_REQUESTS.get(key);
        if (last != null && now - last < 1_000) return;
        FRAME_REQUESTS.put(key, now);
        PacketDistributor.sendToServer(new TerminalReadyPacket(id, (byte) displayId, 0));
    }

    /** Whether phone {@code id} is in the local player's inventory (your own phone, wherever it is drawn). */
    public static boolean isLocalPhone(UUID id) {
        LocalPlayer p = Minecraft.getInstance().player;
        if (p == null) return false;
        var inv = p.getInventory();
        for (int i = 0; i < inv.getContainerSize(); i++) if (id.equals(PhoneItemData.id(inv.getItem(i)))) return true;
        return false;
    }

    public static boolean renderOthers() {
        return PhoneClientConfig.renderOthers();
    }

    /** Send the visibility choices if they changed (or were never sent this session). */
    public static void sendPrefs() {
        if (Minecraft.getInstance().getConnection() == null) return;
        var p = new PhonePackets.Prefs(PhoneClientConfig.share(), PhoneClientConfig.renderOthers());
        if (p.equals(sentPrefs)) return;
        sentPrefs = p;
        PacketDistributor.sendToServer(p);
    }

    // ------------------------------------------------------------ hotkeys

    /** The phone in the main hand, else the off hand, or null. */
    @Nullable
    public static UUID heldPhone() {
        LocalPlayer p = Minecraft.getInstance().player;
        if (p == null) return null;
        UUID id = PhoneItemData.id(p.getItemInHand(InteractionHand.MAIN_HAND));
        return id != null ? id : PhoneItemData.id(p.getItemInHand(InteractionHand.OFF_HAND));
    }

    private static void tick() {
        Minecraft mc = Minecraft.getInstance();
        if (mc.player == null) {
            downPhone = null;
            java.util.Arrays.fill(down, false);
            return;
        }
        sendPrefs();
        UUID phone = heldPhone();
        boolean active = phone != null && mc.screen == null;
        if (downPhone != null && !downPhone.equals(phone)) releaseAll();
        for (int i = 0; i < 6; i++) {
            // isDown only: reading a mapping never takes it from anyone else.
            boolean now = active && SLOTS[i].isDown();
            if (now != down[i]) {
                down[i] = now;
                send(phone, i + 1, now);
                downPhone = now ? phone : downPhone;
            }
        }
        while (SWITCH_APP.consumeClick()) {
            if (active) send(phone, 0, true);
        }
    }

    private static void releaseAll() {
        for (int i = 0; i < 6; i++) {
            if (down[i]) send(downPhone, i + 1, false);
            down[i] = false;
        }
        downPhone = null;
    }

    private static void send(UUID phone, int slot, boolean isDown) {
        if (phone == null || Minecraft.getInstance().getConnection() == null) return;
        hotkeysSent++;
        PacketDistributor.sendToServer(new PhonePackets.Hotkey(phone, slot, isDown));
    }

    /** Hotkey packets sent so far (render checks). */
    public static int hotkeysSent() {
        return hotkeysSent;
    }
}
//?}
