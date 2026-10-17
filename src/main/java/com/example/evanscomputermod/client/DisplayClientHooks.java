package com.example.evanscomputermod.client;

import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.network.ClientPlayout;
import com.example.evanscomputermod.network.ClockSync;
import net.minecraft.client.Minecraft;
import net.neoforged.api.distmarker.Dist;
import net.neoforged.bus.api.SubscribeEvent;
import net.neoforged.fml.common.EventBusSubscriber;
import net.neoforged.neoforge.client.event.ClientPlayerNetworkEvent;
import net.neoforged.neoforge.client.event.ClientTickEvent;
import net.neoforged.neoforge.client.event.RenderFrameEvent;

import java.util.concurrent.ConcurrentLinkedQueue;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;

/**
 * Client threads of the display pipeline (DISPLAY_PIPELINE_SPEC.md Part A5):
 * display packets are inflated, parsed and JPEG-decoded on one
 * {@code ECM-Display-Decode} thread (in arrival order) and acked from there on
 * receipt; only "apply to the display and draw" runs on the render thread,
 * once per rendered frame, where the playout buffer ({@link ClientPlayout})
 * also shows the timed frames that are due.
 */
@EventBusSubscriber(modid = EvansComputerMod.MODID, value = Dist.CLIENT)
public final class DisplayClientHooks {
    private DisplayClientHooks() {}

    private static final ExecutorService DECODE = Executors.newSingleThreadExecutor(r -> {
        Thread t = new Thread(r, "ECM-Display-Decode");
        t.setDaemon(true);
        return t;
    });
    private static final ConcurrentLinkedQueue<Runnable> MAIN = new ConcurrentLinkedQueue<>();

    /** Run on the decode thread, after everything handed to it before. */
    public static void decode(Runnable r) {
        DECODE.execute(() -> {
            try {
                r.run();
            } catch (Throwable t) {
                EvansComputerMod.LOGGER.debug("display decode failed", t);
            }
        });
    }

    /** Run on the render thread at the next frame (or tick, when nothing renders). */
    public static void toMain(Runnable r) {
        MAIN.add(r);
    }

    private static void drain() {
        Runnable r;
        while ((r = MAIN.poll()) != null) {
            try {
                r.run();
            } catch (Throwable t) {
                EvansComputerMod.LOGGER.debug("display apply failed", t);
            }
        }
    }

    @SubscribeEvent
    public static void onFrame(RenderFrameEvent.Pre event) {
        if (Minecraft.getInstance().level == null) {
            MAIN.clear();
            return;
        }
        drain();
        ClientPlayout.tick(ClockSync.clientNowUs());
    }

    @SubscribeEvent
    public static void onTick(ClientTickEvent.Post event) {
        if (Minecraft.getInstance().level == null) return;
        drain();
        ClientPlayout.tick(ClockSync.clientNowUs());
    }

    @SubscribeEvent
    public static void onLoggingOut(ClientPlayerNetworkEvent.LoggingOut event) {
        MAIN.clear();
        ClientPlayout.clear();
    }
}
