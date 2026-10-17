package com.example.evanscomputermod.card.client;

//? if <=1.21.1 {
import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.card.CardFaces;
import com.example.evanscomputermod.card.CardKind;
import com.mojang.blaze3d.platform.NativeImage;
import net.minecraft.client.Minecraft;
import net.minecraft.client.renderer.texture.DynamicTexture;
import net.minecraft.resources.ResourceLocation;

import java.util.Iterator;
import java.util.LinkedHashMap;
import java.util.Map;

/**
 * Client: a card's painted faces as one texture, the front above the back
 * ({@code w x 2h} texels), composed from its {@link CardFaces} with the kind's fixed
 * features (chip pad, stripe) drawn over the masked texels. Cached by the faces'
 * content hash, so a bank's two hundred identical cards share one texture; textures
 * nobody drew for a minute are released.
 */
public final class CardFaceTexture {
    private static final int PAD = 0xFFD4AF37, PAD_LINE = 0xFFB8962E, STRIPE = 0xFF1B1B1F, STRIPE_CORRUPT = 0xFF3A2A2A;

    private record Key(CardKind kind, long hash, boolean corrupted) {}

    private static final Map<Key, CardFaceTexture> CACHE = new LinkedHashMap<>();
    private static long frame;

    private final ResourceLocation location;
    private final DynamicTexture texture;
    private long lastUsed;

    private CardFaceTexture(Key key, CardFaces faces) {
        CardKind k = key.kind();
        int w = k.faceWidth, h = k.faceHeight;
        location = ResourceLocation.fromNamespaceAndPath(EvansComputerMod.MODID,
                "dynamic/card_" + k.id() + "_" + Long.toHexString(key.hash()) + (key.corrupted() ? "_x" : ""));
        texture = new DynamicTexture(w, 2 * h, false);
        NativeImage img = texture.getPixels();
        if (img != null) {
            boolean[] mf = k.mask(true), mb = k.mask(false);
            for (int y = 0; y < h; y++)
                for (int x = 0; x < w; x++) {
                    int i = y * w + x;
                    int front = mf[i] ? pad(k, x, y) : faces.argb(true, i);
                    int back = mb[i] ? (key.corrupted() && (x + y) % 3 == 0 ? STRIPE_CORRUPT : STRIPE) : faces.argb(false, i);
                    img.setPixelRGBA(x, y, abgr(front));
                    img.setPixelRGBA(x, y + h, abgr(back));
                }
        }
        texture.upload();
        Minecraft.getInstance().getTextureManager().register(location, texture);
    }

    /** The contact pad: gold with a dark cross of contact lines. */
    private static int pad(CardKind k, int x, int y) {
        int x0 = k == CardKind.SIM ? 1 : 3, y0 = k == CardKind.SIM ? 3 : (k.faceHeight - 5) / 2;
        int lx = x - x0, ly = y - y0;
        return lx == 3 || ly == 2 ? PAD_LINE : PAD;
    }

    private static int abgr(int argb) {
        return (argb & 0xFF00FF00) | ((argb >>> 16) & 0xFF) | ((argb & 0xFF) << 16);
    }

    /** The texture for {@code faces} on a {@code kind} card (null faces: the blank look). Render thread. */
    public static ResourceLocation get(CardKind kind, CardFaces faces, boolean stripeCorrupted) {
        CardFaces f = faces != null && faces.fits(kind) ? faces : CardFaces.blank(kind);
        Key key = new Key(kind, f.contentHash(), stripeCorrupted);
        CardFaceTexture t = CACHE.get(key);
        if (t == null) {
            t = new CardFaceTexture(key, f);
            CACHE.put(key, t);
        }
        t.lastUsed = frame;
        return t.location;
    }

    /** Once per frame: forget textures nobody drew for a while. */
    public static void endFrame() {
        frame++;
        if (frame % 600 != 0) return;
        for (Iterator<CardFaceTexture> it = CACHE.values().iterator(); it.hasNext(); ) {
            CardFaceTexture t = it.next();
            if (frame - t.lastUsed > 1200) {
                Minecraft.getInstance().getTextureManager().release(t.location);
                t.texture.close();
                it.remove();
            }
        }
    }

    public static int cached() {
        return CACHE.size();
    }
}
//?}
