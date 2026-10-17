package com.example.evanscomputermod.phone;

//? if <=1.21.1 {
import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.api.IFramebufferDisplay;
import com.example.evanscomputermod.api.IRedstoneProvider;
import com.example.evanscomputermod.api.IWorldAccess;
import com.example.evanscomputermod.api.IComputerHost;
import com.example.evanscomputermod.api.module.IComputerModule;
import com.example.evanscomputermod.computer.ComputerHost;
import com.example.evanscomputermod.computer.ComputerInstance;
import com.example.evanscomputermod.computer.ComputerRegistry;
import com.example.evanscomputermod.computer.DisplaySyncHub;
import com.example.evanscomputermod.computer.TerminalDisplay;
import com.example.evanscomputermod.computer.display.PanelTier;
import com.example.evanscomputermod.computer.peripheral.PeripheralEventBus;
import com.example.evanscomputermod.computer.peripheral.PeripheralHub;
import com.example.evanscomputermod.module.InstalledModules;
import com.example.evanscomputermod.module.ModuleBays;
import com.example.evanscomputermod.network.TerminalDeltaPacket;
import com.example.evanscomputermod.radio.api.Pose;
import com.example.evanscomputermod.radio.cellular.SimCard;
import com.example.evanscomputermod.radio.cellular.SimCardItem;
import com.example.evanscomputermod.radio.wifi.WifiRadio;
import com.example.evanscomputermod.wasm.WasmManager;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.network.chat.Component;
import net.minecraft.server.MinecraftServer;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.server.level.ServerPlayer;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.level.Level;
import net.minecraft.world.phys.Vec3;
import net.neoforged.neoforge.common.util.FakePlayer;
import net.neoforged.neoforge.network.PacketDistributor;
import org.jetbrains.annotations.Nullable;

import java.util.ArrayList;
import java.util.Arrays;
import java.util.Collection;
import java.util.HashSet;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.UUID;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;

/**
 * The held-item host of one ePhone computer (one per computer UUID per server;
 * duplicated phone items share it). Server-owned, like {@link ComputerHost}:
 * {@link PhoneServer} finds the phone in a player's hotbar or off hand every
 * tick and calls {@link #tick} with where it is (or null).
 *
 * <p>While the phone is in a hotbar or hand the host is attached to the
 * computer's {@link ComputerHost} and registered by UUID (display and input
 * packets find it), its modules are live with the holder's eye as their pose,
 * and the battery drains. Leaving the hotbar puts the computer to sleep, takes
 * the modules out of the world and, once the sleep has landed, detaches the
 * host: the computer stays in memory, paused, until the phone is picked up.
 * Logging out (or an empty battery) stops the computer; it cold-boots next time.
 *
 * <p>Displays: {@link IComputerHost#DISPLAY_OWN} is streamed to the holder only.
 * {@link #DISPLAY_MIRROR} is the same picture streamed, at a lower rate, to
 * nearby players when visibility allows (server max FULL, holder shares,
 * onlooker renders others). Onlookers may ack the mirror but never send input.
 */
public final class PhoneHost implements IComputerHost {

    /** Display id of the low-rate mirror of the phone's screen sent to onlookers. */
    public static final int DISPLAY_MIRROR = 2;
    /** Ticks a stopping phone waits for its sleep to land before detaching anyway. */
    private static final int DETACH_TIMEOUT_TICKS = 40;
    /** Status packets go to players this close to the holder (they render the phone in its hand). */
    private static final double STATUS_RANGE = 48;

    private static final ExecutorService LOADER = Executors.newSingleThreadExecutor(r -> {
        Thread t = new Thread(r, "ePhone-Loader");
        t.setDaemon(true);
        return t;
    });

    private final MinecraftServer server;
    private final UUID id;
    private final TerminalDisplay display = new TerminalDisplay();
    private final DisplaySyncHub holderSync = new DisplaySyncHub(DISPLAY_OWN);
    private final DisplaySyncHub mirrorSync = new DisplaySyncHub(DISPLAY_MIRROR);
    private final PhonePeripheral phonePeripheral = new PhonePeripheral(this);
    /** The SIM tray as SIM access (peripheral {@code sim}): Cellular v2's {@code celld} authenticates through it. */
    private final com.example.evanscomputermod.card.SimPeripheral simPeripheral =
            new com.example.evanscomputermod.card.SimPeripheral(com.example.evanscomputermod.card.SimPeripheral.ofCard(this::sim));
    /** The phone's own speaker (peripheral {@code speaker}, {@code /dev/audio}): heard around the holder. */
    private final com.example.evanscomputermod.speaker.SpeakerAudio speakerAudio = new com.example.evanscomputermod.speaker.SpeakerAudio();
    private final com.example.evanscomputermod.speaker.ImaAdpcm speakerEncoder = new com.example.evanscomputermod.speaker.ImaAdpcm();
    private int speakerSeq;
    private final com.example.evanscomputermod.speaker.SpeakerPeripheral speaker = new com.example.evanscomputermod.speaker.SpeakerPeripheral(
            new com.example.evanscomputermod.speaker.SpeakerPeripheral.Owner() {
                @Override public com.example.evanscomputermod.speaker.SpeakerAudio audio() { return speakerAudio; }
                @Override public void markVolumeChanged() {}
                @Override public boolean playSound(String sound, float volume, float pitch) {
                    ServerPlayer p = holder;
                    var rl = net.minecraft.resources.ResourceLocation.tryParse(sound);
                    if (p == null || rl == null) return false;
                    var event = net.minecraft.core.registries.BuiltInRegistries.SOUND_EVENT.getOptional(rl);
                    if (event.isEmpty()) return false;
                    p.serverLevel().playSound(null, p.blockPosition(), event.get(), net.minecraft.sounds.SoundSource.PLAYERS,
                            volume * speakerAudio.volume() / 100f, pitch);
                    return true;
                }
            });
    /**
     * The phone's own microphone (peripheral {@code microphone}, {@code /dev/mic}): at the holder's
     * hand, so it hears the holder close up and everything around them.
     */
    private final com.example.evanscomputermod.voice.MicAudio micAudio = new com.example.evanscomputermod.voice.MicAudio(
            (start, n, rate, out) -> {
                // The headphones layer: with AirPods connected and worn, /dev/mic is what their microphones sent.
                if (this.bluetooth.micRouted()) {
                    this.bluetooth.renderMic(start, n, rate, out);
                    return;
                }
                com.example.evanscomputermod.voice.VoiceServer.Spot s = micSpot();
                if (s != null) com.example.evanscomputermod.voice.SoundField.get().render(s.dimension(), s.x(), s.y(), s.z(), start, n, rate, out);
            });
    private final com.example.evanscomputermod.voice.MicrophonePeripheral mic = new com.example.evanscomputermod.voice.MicrophonePeripheral(
            new com.example.evanscomputermod.voice.MicrophonePeripheral.Owner() {
                @Override public com.example.evanscomputermod.voice.MicAudio mic() { return micAudio; }
                @Override public void markGainChanged() {}
            });

    /**
     * The phone's Bluetooth radio (peripheral {@code bluetooth}): AirPods pair and connect here, and
     * while they are connected and worn the phone's speaker and microphone are theirs (docs/BLUETOOTH.md).
     */
    private com.example.evanscomputermod.bluetooth.PhoneBluetooth bluetooth;
    private com.example.evanscomputermod.bluetooth.BluetoothPeripheral btPeripheral;

    /**
     * Where the phone's microphone is: a little below the holder's mouth, or the mouth of whoever
     * wears the connected AirPods; null while nobody holds it.
     */
    @Nullable
    private com.example.evanscomputermod.voice.VoiceServer.Spot micSpot() {
        if (bluetooth.micRouted()) {
            ServerPlayer w = com.example.evanscomputermod.bluetooth.Headsets.wearerOf(bluetooth.connected());
            if (w != null) return com.example.evanscomputermod.voice.VoiceServer.mouth(w);
        }
        ServerPlayer p = holder;
        if (p == null) return null;
        var eye = p.getEyePosition();
        return new com.example.evanscomputermod.voice.VoiceServer.Spot(p.level().dimension().location().toString(), eye.x, eye.y - 0.4, eye.z);
    }

    /** The PhoneUi the status source was last given to (PhoneApps). */
    private Object statusSource;

    private volatile PhoneHardware hardware = PhoneHardware.EMPTY;
    private boolean hardwareDirty;

    @Nullable private volatile ServerPlayer holder;
    @Nullable private UUID holderId;
    private String holderName = "";
    private ItemStack stack = ItemStack.EMPTY;
    private boolean held;

    private volatile PhoneState state = PhoneState.OFF;
    private volatile boolean attached;
    /** Display passes on the pump (DISPLAY_PIPELINE_SPEC.md Part A1): the holder's screen, and the onlookers' mirror. */
    private final com.example.evanscomputermod.computer.pipeline.DisplayPump.Lane syncLane =
            new com.example.evanscomputermod.computer.pipeline.DisplayPump.Lane("phone", this::syncNow);
    private final com.example.evanscomputermod.computer.pipeline.DisplayPump.Lane mirrorLane =
            new com.example.evanscomputermod.computer.pipeline.DisplayPump.Lane("phone-mirror", this::mirrorPass);
    /** Who sees and may use the phone, as of the last tick (Part A2). */
    private volatile com.example.evanscomputermod.computer.pipeline.ViewerSnapshot viewerSnapshot =
            com.example.evanscomputermod.computer.pipeline.ViewerSnapshot.EMPTY;
    private volatile boolean booting;
    /** Ticks left until a detach is forced; -1 = no detach pending. */
    private int detachIn = -1;
    /** Wall time (nanos) at which a delayed sleep (lifecycle grace) happens; 0 = none. */
    private long sleepAtNanos;
    private boolean deadNotified;

    private double battery = 100;
    private int writtenCenti = -1;
    private long ticks;

    private final Set<UUID> mirrorViewers = new HashSet<>();
    /** The onlookers' player objects, for the viewer snapshot (scenario onlookers aren't in the player list). */
    private final java.util.Map<UUID, ServerPlayer> mirrorPlayers = new java.util.HashMap<>();
    private boolean mirrorPending;
    private long lastMirrorTick;
    private PhoneConfig.Visibility visibility = PhoneConfig.Visibility.OFF;
    private PhoneState sentState;
    private PhoneConfig.Visibility sentVisibility;
    private int sentCenti = -1;

    /** Display packets sent per player (tests). */
    private final Map<UUID, Integer> packetsTo = new ConcurrentHashMap<>();
    private final List<String> transitions = new java.util.concurrent.CopyOnWriteArrayList<>();

    private final PeripheralHub peripherals = new PeripheralHub(new PeripheralHub.Owner() {
        @Override
        public Level level() {
            ServerPlayer p = holder;
            return p != null ? p.serverLevel() : null;
        }

        @Override
        public BlockPos pos() {
            ServerPlayer p = holder;
            return p != null ? p.blockPosition() : BlockPos.ZERO;
        }

        @Override
        public Direction facing() {
            ServerPlayer p = holder;
            return p != null ? p.getDirection() : Direction.NORTH;
        }

        @Override
        public UUID computerId() {
            return id;
        }

        @Override
        @Nullable
        public PeripheralEventBus events() {
            ComputerInstance c = instance();
            return c != null ? c.getPeripheralEvents() : null;
        }
    });

    /** Paired Wireless Controllers (its built-in receiver): {@code controller_1..4} while the phone runs. */
    private final com.example.evanscomputermod.controller.WirelessControllerHub controllers =
            new com.example.evanscomputermod.controller.WirelessControllerHub(peripherals);

    private final ModuleBays bays = new ModuleBays(new ModuleBays.Owner() {
        @Override
        public Level level() {
            ServerPlayer p = holder;
            return p != null ? p.serverLevel() : null;
        }

        @Override
        public Pose pose() {
            return PhoneHost.this.pose();
        }

        @Override
        public Direction facing() {
            ServerPlayer p = holder;
            return p != null ? p.getDirection() : Direction.NORTH;
        }

        @Override
        public boolean isAlive() {
            return attached;
        }

        @Override
        public void onBaysChanged() {
            hardwareDirty = true;
        }

        @Override
        public void onModuleChanged(String slotName, @Nullable IComputerModule module) {
            // The SIM tray is the modem's SIM (set before attaching: a framed modem then brings no SIM slot of its own).
            if (module instanceof com.example.evanscomputermod.radio.cellular.FramedModemModule f) f.setSimProvider(PhoneHost.this::sim);
            peripherals.setModule(slotName, module);
        }

        @Override
        public String slotName(int slot) {
            return "module_" + (slot + 1);
        }
    });

    PhoneHost(MinecraftServer server, UUID id) {
        this.server = server;
        this.id = id;
        this.bluetooth = new com.example.evanscomputermod.bluetooth.PhoneBluetooth(id, "ePhone");
        this.btPeripheral = new com.example.evanscomputermod.bluetooth.BluetoothPeripheral(bluetooth.link());
        speakerAudio.setWriteListener(() -> PhoneApps.audioWritten(this));
        speakerAudio.setStatusExtra(() -> bluetooth.audioRouted() ? "headphones 1\n" : "headphones 0\n");
        speakerAudio.setLabel(() -> bluetooth.audioRouted() ? "AirPods" : "ePhone speaker");
        micAudio.setReadListener(() -> PhoneApps.micRead(this));
        // A double-tap on the connected AirPods plays or pauses the phone's media session.
        bluetooth.setButtonSink(g -> {
            if (g == com.example.evanscomputermod.bluetooth.BtFrame.GESTURE_DOUBLE_TAP) PhoneApps.mediaToggle(this);
        });
    }

    // ------------------------------------------------------------ state machine (server thread)

    /** Where the phone is this tick: who has it, the stack in their inventory, and whether it is in a hand. */
    public record Holding(ServerPlayer player, ItemStack stack, boolean held) {}

    /** One server tick. {@code where} is null when no player has the phone in a hotbar slot or hand. */
    void tick(@Nullable Holding where) {
        ticks++;
        PhoneState want = PhoneState.OFF;
        if (where != null) {
            if (!attached) battery = PhoneItemData.batteryPercent(where.stack());
            holder = where.player();
            holderId = where.player().getUUID();
            holderName = where.player().getGameProfile().getName();
            stack = where.stack();
            held = where.held();
            want = held ? PhoneState.AWAKE : PhoneState.ASLEEP;
            if (PhoneConfig.batteryEnabled() && battery <= 0) {
                if (instance() != null || attached || booting) powerOff("battery empty");
                if (!deadNotified && held) {
                    where.player().displayClientMessage(Component.translatable("message.evanscomputermod.ephone.battery_empty"), true);
                }
                deadNotified = true;
                want = PhoneState.OFF;
            } else {
                deadNotified = false;
            }
        }
        apply(want);

        if (sleepAtNanos != 0 && System.nanoTime() >= sleepAtNanos) {
            sleepAtNanos = 0;
            if (state != PhoneState.AWAKE) sleepComputer();
        }
        if (detachIn >= 0) {
            ComputerInstance c = instance();
            boolean landed = c == null || sleepAtNanos == 0 && c.isAsleep() && awaitAsleepNow(c);
            if (landed || --detachIn <= 0) detach();
        }
        if (!attached) return;

        bays.tick();
        if (!holderName.isEmpty()) bluetooth.link().setName(holderName + "'s ePhone");
        bluetooth.tick(pose(), micAudio.listening());
        if (bluetooth.link().takeChanged()) writeBluetoothToStack();
        followAudioRoute();
        ComputerInstance running = instance();
        if (running != null) running.tickSync();   // text console changes reach the holder from here
        drain();
        if (hardwareDirty) {
            hardwareDirty = false;
            writeModulesToStack();
        }
        if (ticks % 20 == 0) writeBatteryToStack(false);
        updateMirror();
        publishViewers();
        sendStatus(false);
        playSpeaker();
        controllers.tick(System.currentTimeMillis());
        PhoneApps.tick(this);
    }

    private static boolean awaitAsleepNow(ComputerInstance c) {
        try {
            return c.awaitAsleep(0);
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
            return false;
        }
    }

    private void apply(PhoneState want) {
        PhoneState from = state;
        if (want == from) {
            if (want != PhoneState.OFF) ensureBooted();
            return;
        }
        if (want != PhoneState.OFF) attach();
        long grace = from == PhoneState.AWAKE ? PhoneLifecycle.willSleep(this, want) : 0;
        state = want;
        switch (want) {
            case AWAKE -> {
                sleepAtNanos = 0;
                ensureBooted();
                ComputerInstance c = instance();
                if (c != null) c.wake();
            }
            case ASLEEP -> {
                ensureBooted();
                sleepSoon(grace);
            }
            case OFF -> {
                sleepSoon(grace);
                stopModules();
                detachIn = DETACH_TIMEOUT_TICKS;
            }
        }
        transitions.add(from.id() + ">" + want.id());
        PhoneLifecycle.stateChanged(this, from, want);
        sendStatus(true);
    }

    private void sleepSoon(long graceMs) {
        if (graceMs > 0) {
            sleepAtNanos = System.nanoTime() + graceMs * 1_000_000L;
        } else {
            sleepAtNanos = 0;
            sleepComputer();
        }
    }

    private void sleepComputer() {
        ComputerInstance c = instance();
        // Switched off (about to unload): a wake lock doesn't keep it running; asleep, a call's lock may.
        if (c != null && state == PhoneState.OFF) c.sleepIgnoringWakeLocks();
        else if (c != null) c.sleep();
    }

    /** Hook the host into the computer: attach, register by UUID, load the hardware from the stack, start modules. */
    private void attach() {
        detachIn = -1;
        ComputerHost owner = ComputerHost.get(server, id);
        if (attached) {
            if (!bays.isLoaded()) startModules();
            return;
        }
        attached = true;
        owner.attach(this);
        ComputerRegistry.register(this);
        hardware = PhoneHardware.of(stack);
        startModules();
        applyPanel();
        forceNextKeyframe();
    }

    private void startModules() {
        List<ItemStack> slots = new ArrayList<>(List.of(hardware.module1(), hardware.module2(), ItemStack.EMPTY, ItemStack.EMPTY));
        bays.restore(new InstalledModules(1, slots));
        bays.load();
        peripherals.setWireless(PhonePeripheral.NAME, phonePeripheral);
        peripherals.setWireless("speaker", speaker);
        peripherals.setWireless("microphone", mic);
        com.example.evanscomputermod.voice.VoiceServer.addMic(micAudio, this::micSpot);
        if (!stack.isEmpty()) bluetooth.load(stack.getOrDefault(net.minecraft.core.component.DataComponents.CUSTOM_DATA,
                net.minecraft.world.item.component.CustomData.EMPTY).copyTag().getCompound(BLUETOOTH_KEY));
        peripherals.setWireless(com.example.evanscomputermod.bluetooth.BluetoothPeripheral.TYPE, btPeripheral);
        peripherals.setWireless(com.example.evanscomputermod.card.SimPeripheral.TYPE, simPeripheral);
    }

    private void stopModules() {
        if (bays.isLoaded()) {
            writeModulesToStack();
            bays.unload();
        }
        controllers.disconnectAll();
        peripherals.detachAll();
        com.example.evanscomputermod.voice.VoiceServer.removeMic(micAudio);
        bluetooth.stop();
    }

    /** Where the phone item keeps its Bluetooth pairings (custom data). */
    private static final String BLUETOOTH_KEY = "ecm_bluetooth";

    private void writeBluetoothToStack() {
        if (stack.isEmpty() || PhoneItemData.id(stack) == null) return;
        net.minecraft.nbt.CompoundTag t = bluetooth.save();
        net.minecraft.world.item.component.CustomData.update(net.minecraft.core.component.DataComponents.CUSTOM_DATA, stack,
                root -> root.put(BLUETOOTH_KEY, t));
    }

    /** The speaker's volume and the AirPods' are separate (the Volume Mixer's master strip is whichever plays). */
    private int otherRouteVolume = 100;
    private boolean lastRouted;

    private void followAudioRoute() {
        boolean routed = bluetooth.audioRouted();
        if (routed == lastRouted) return;
        lastRouted = routed;
        int now = speakerAudio.volume();
        speakerAudio.setVolume(otherRouteVolume);
        otherRouteVolume = now;
    }

    /** The phone's Bluetooth radio (tests, debug). */
    public com.example.evanscomputermod.bluetooth.PhoneBluetooth bluetooth() {
        return bluetooth;
    }

    /** The sleep landed (or timed out): let go of the computer. It stays in memory, paused. */
    private void detach() {
        detachIn = -1;
        if (!attached) return;
        stopModules();
        writeBatteryToStack(true);
        attached = false;
        ComputerHost owner = ComputerHost.get(server, id);
        if (owner.attachment() == this) owner.attach(null);
        ComputerRegistry.unregister(id, this);
        holderSync.reset();
        mirrorSync.reset();
        viewerSnapshot = com.example.evanscomputermod.computer.pipeline.ViewerSnapshot.EMPTY;
        mirrorViewers.clear();
        mirrorPlayers.clear();
        sendStatus(true);
        holder = null;
        stack = ItemStack.EMPTY;
    }

    /** Boot the computer if nothing runs or boots for this UUID yet. */
    private void ensureBooted() {
        ComputerHost owner = ComputerHost.get(server, id);
        if (owner.instance() != null || booting || owner.isBooting()) return;
        if (!owner.beginBoot()) return;
        booting = true;
        EvansComputerMod.LOGGER.info("Booting ePhone {}", id);
        CompletableFuture.supplyAsync(() -> {
            try {
                ComputerInstance c = new ComputerInstance(owner, new byte[0][]);
                c.loadModule("terminal_os");
                return c;
            } catch (WasmManager.WasmExecutionException e) {
                throw new RuntimeException(e);
            }
        }, LOADER).whenComplete((c, err) -> server.execute(() -> onBooted(owner, c, err)));
    }

    private void onBooted(ComputerHost owner, @Nullable ComputerInstance c, @Nullable Throwable err) {
        owner.endBoot();
        booting = false;
        if (err != null || c == null) {
            EvansComputerMod.LOGGER.error("ePhone {} failed to boot", id, err);
            return;
        }
        if (owner.instance() != null) {
            c.close();
            return;
        }
        owner.track(c);
        try {
            c.executeMain();
            c.startWorkerThread();
        } catch (WasmManager.WasmExecutionException e) {
            EvansComputerMod.LOGGER.error("ePhone {} kernel failed to start", id, e);
            return;
        }
        applyPanel();
        if (state != PhoneState.AWAKE) c.sleep();
        forceNextKeyframe();
    }

    /**
     * Stop the computer (logout of the holder, empty battery): it is closed and
     * cold-boots next time. App state survives only through the app platform's
     * save/restore (P4).
     */
    void powerOff(String why) {
        PhoneState from = state;
        sleepAtNanos = 0;
        if (state != PhoneState.OFF) {
            state = PhoneState.OFF;
            transitions.add(from.id() + ">off(" + why + ")");
            PhoneLifecycle.stateChanged(this, from, PhoneState.OFF);
        }
        detach();
        ComputerHost owner = ComputerHost.get(server, id);
        ComputerInstance c = owner.instance();
        if (c != null) {
            c.interrupt();
            c.close();
        }
        owner.forget();
        EvansComputerMod.LOGGER.info("ePhone {} stopped: {}", id, why);
        PhoneLifecycle.stopped(this, why);
    }

    // ------------------------------------------------------------ hardware

    /** Panel, modules and SIM as fitted now. */
    public PhoneHardware hardware() {
        return hardware;
    }

    /**
     * The slot screen changed the phone's hardware: write it to the stack and
     * apply it live (panel hot-plug resizes the console, modules restart, the
     * SIM tray is read by modems every tick).
     */
    void setHardware(PhoneHardware h) {
        PhoneHardware old = hardware;
        hardware = h;
        if (!stack.isEmpty()) stack.set(PhoneContent.HARDWARE.get(), h);
        if (!attached) return;
        if (!ItemStack.matches(old.module1(), h.module1()) || !ItemStack.matches(old.module2(), h.module2())) {
            bays.unload();
            startModules();
        }
        if (!ItemStack.matches(old.panel(), h.panel())) applyPanel();
    }

    private void writeModulesToStack() {
        InstalledModules snap = bays.snapshot();
        PhoneHardware h = hardware.withModule(0, snap.slots().get(0)).withModule(1, snap.slots().get(1));
        hardware = h;
        if (!stack.isEmpty() && PhoneItemData.id(stack) != null) stack.set(PhoneContent.HARDWARE.get(), h);
    }

    @Nullable
    private SimCard sim() {
        return SimCardItem.sim(hardware.sim());
    }

    /** Resize the console to the panel's portrait grid; re-check program framebuffers. */
    private void applyPanel() {
        ComputerInstance c = instance();
        if (c == null) return;
        PanelTier tier = hardware.tier();
        if (tier != null) {
            var mode = tier.modes().phone();
            c.requestConsoleResize(mode.textCols(), mode.textRows());
        }
        c.displayDevice(ComputerInstance.GFX_TARGET_TERMINAL).enforceLimits();
        // The GUI follows the panel (hot-plug): the shell gets DisplayChanged and lays out again.
        c.getUiManager().panelChanged();
        forceNextKeyframe();
    }

    // ------------------------------------------------------------ battery

    private void drain() {
        if (!PhoneConfig.batteryEnabled()) {
            battery = 100;
            return;
        }
        double perMinute = switch (state) {
            case AWAKE -> PhoneConfig.awakePerMinute() * PhoneConfig.tierFactor(hardware.tier());
            case ASLEEP -> PhoneConfig.asleepPerMinute();
            case OFF -> PhoneConfig.offPerMinute();
        };
        if (state != PhoneState.OFF) perMinute += activeRadios() * PhoneConfig.radioPerMinute();
        battery = Math.max(0, battery - perMinute / 1200.0);
        if (battery <= 0 && state != PhoneState.OFF) {
            writeBatteryToStack(true);
            ServerPlayer p = holder;
            if (p != null) p.displayClientMessage(Component.translatable("message.evanscomputermod.ephone.battery_empty"), true);
            deadNotified = true;
            powerOff("battery empty");
        }
    }

    /** Radio modules drawing power: Wi-Fi switched on, modems on a cell (celld attached). */
    public int activeRadios() {
        int n = 0;
        for (int i = 0; i < 2; i++) {
            IComputerModule m = bays.getModule(i);
            if (m instanceof WifiRadio w && w.wifiActive()) n++;
            else if (m instanceof com.example.evanscomputermod.radio.cellular.FramedModemModule && PhoneApps.cellAttached(this)) n++;
        }
        return n;
    }

    private void writeBatteryToStack(boolean force) {
        int centi = (int) Math.round(battery * 100);
        if (stack.isEmpty() || PhoneItemData.id(stack) == null) return;
        if (!force && centi == writtenCenti) return;
        writtenCenti = centi;
        stack.set(PhoneContent.BATTERY.get(), centi);
    }

    /** Charge in percent (100 when the server has batteries off). */
    public double batteryPercent() {
        return PhoneConfig.batteryEnabled() ? battery : 100;
    }

    /** Test hook: set the charge of an attached phone. */
    public void setBatteryForTest(double percent) {
        battery = Math.max(0, Math.min(100, percent));
        writeBatteryToStack(true);
    }

    // ------------------------------------------------------------ visibility, mirror and status

    /** What onlookers may see now: the server's max, lowered by the holder's share toggle; dark unless awake. */
    public PhoneConfig.Visibility visibility() {
        return visibility;
    }

    private PhoneConfig.Visibility computeVisibility() {
        if (state != PhoneState.AWAKE || !attached) return PhoneConfig.Visibility.OFF;
        PhoneConfig.Visibility max = PhoneConfig.maxVisibility();
        UUID h = holderId;
        if (h != null && !PhoneServer.prefs(h).share() && max == PhoneConfig.Visibility.FULL) return PhoneConfig.Visibility.GLOW;
        return max;
    }

    private void updateMirror() {
        visibility = computeVisibility();
        int interval = PhoneConfig.mirrorIntervalTicks();
        if (ticks % interval == 0) {
            mirrorViewers.clear();
            mirrorPlayers.clear();
            ServerPlayer h = holder;
            if (visibility == PhoneConfig.Visibility.FULL && h != null) {
                double r = PhoneConfig.mirrorRange();
                for (ServerPlayer p : PhoneServer.playersNear(h.serverLevel(), h.position(), r)) {
                    if (p == h || p.getUUID().equals(holderId)) continue;
                    if (!PhoneServer.prefs(p.getUUID()).renderOthers()) continue;
                    mirrorViewers.add(p.getUUID());
                    mirrorPlayers.put(p.getUUID(), p);
                }
            }
            mirrorSync.retainViewers(mirrorViewers);
            if (!mirrorViewers.isEmpty()) syncMirror();
        } else if (mirrorPending && ticks - lastMirrorTick >= interval) {
            syncMirror();
        }
    }

    private void syncMirror() {
        mirrorPending = false;
        lastMirrorTick = ticks;
        if (mirrorViewers.isEmpty()) return;
        mirrorLane.request();
    }

    /** The holder's display: the phone's screen, its program's presentation queue, lossy unless Tier 1. */
    private DisplaySyncHub.Source holderSource() {
        ComputerInstance c = instance();
        var device = c == null ? null : c.displayDevice(ComputerInstance.GFX_TARGET_TERMINAL);
        var tier = displayTier(DISPLAY_OWN);
        var d = display;
        return new DisplaySyncHub.Source() {
            @Override public com.example.evanscomputermod.computer.TerminalDisplay display() { return d; }
            @Override public com.example.evanscomputermod.computer.display.PresentQueue queue() {
                return device == null ? null : device.queue();
            }
            @Override public int hint() { return device == null ? 0 : device.hint(); }
            @Override public boolean lossyAllowed() {
                return tier != null && tier != com.example.evanscomputermod.computer.display.PanelTier.PHOSPHOR;
            }
        };
    }

    /** The mirror's pass, on the display pump, to the onlookers of the last tick. */
    private void mirrorPass() {
        List<UUID> to = viewerSnapshot.recipients(DISPLAY_MIRROR);
        if (to.isEmpty()) return;
        ComputerInstance ci = instance();
        if (ci != null) ci.getUiManager().notePixelDemand(); // onlookers get pixels even when the holder draws the scene
        try {
            mirrorSync.sync(id, display, to, this::send);
        } catch (Exception e) {
            EvansComputerMod.LOGGER.debug("ePhone mirror sync failed", e);
            mirrorSync.forceKeyframe();
        }
    }

    /** Once per tick (server thread): the holder and the onlookers, for the pump, acks and input. */
    private void publishViewers() {
        var b = com.example.evanscomputermod.computer.pipeline.ViewerSnapshot.builder(ticks);
        ServerPlayer h = holder;
        if (h != null && attached) {
            if (state == PhoneState.AWAKE) b.recipient(DISPLAY_OWN, h);
            b.interact(DISPLAY_OWN, h.getUUID());
            if (state == PhoneState.AWAKE) b.input(DISPLAY_OWN, h.getUUID());
            for (UUID m : mirrorViewers) {
                ServerPlayer p = mirrorPlayers.get(m);
                if (p == null) continue;
                b.recipient(DISPLAY_MIRROR, p);
                b.interact(DISPLAY_MIRROR, m);
            }
        }
        var next = b.build();
        var prev = viewerSnapshot;
        viewerSnapshot = next;
        if (!next.sameRecipients(prev)) syncToClients();
    }

    @Override
    public com.example.evanscomputermod.computer.pipeline.ViewerSnapshot viewerSnapshot() {
        return viewerSnapshot;
    }

    /** Status (state, visibility, battery) for the holder and nearby players, on change or every second. */
    private void sendStatus(boolean force) {
        int centi = (int) Math.round(batteryPercent() * 100);
        boolean changed = state != sentState || visibility != sentVisibility || centi / 100 != sentCenti / 100;
        if (!force && !changed && ticks % 20 != 0) return;
        sentState = state;
        sentVisibility = visibility;
        sentCenti = centi;
        ServerPlayer h = holder;
        if (h == null) return;
        var packet = new PhonePackets.Status(id, state.ordinal(), visibility.ordinal(), centi);
        for (ServerPlayer p : PhoneServer.playersNear(h.serverLevel(), h.position(), STATUS_RANGE)) {
            if (!(p instanceof FakePlayer)) PacketDistributor.sendToPlayer(p, packet);
        }
    }

    // ------------------------------------------------------------ IComputerHost

    @Override
    public UUID getComputerId() {
        return id;
    }

    @Override
    public MinecraftServer getServer() {
        return server;
    }

    @Override
    public void markDirty() {
        hardwareDirty = true;
    }

    /** Stream the holder's screen: a pass on the display pump, never the server thread. */
    @Override
    public void syncToClients() {
        syncLane.request();
    }

    /** The holder's pass (display pump); the mirror keeps its own low rate, set by the tick. */
    private void syncNow() {
        if (!attached) return;
        List<UUID> own = viewerSnapshot.recipients(DISPLAY_OWN);
        if (!own.isEmpty()) {
            UUID h = own.get(0);
            if (sceneSync(h)) return;
            try {
                ComputerInstance ci = instance();
                if (ci != null) ci.getUiManager().notePixelDemand();
                holderSync.setWaker(syncLane::requestIn);
                holderSync.sync(id, holderSource(), own, this::send);
            } catch (Exception e) {
                EvansComputerMod.LOGGER.debug("ePhone display sync failed", e);
                holderSync.forceKeyframe();
            }
        }
        if (!viewerSnapshot.recipients(DISPLAY_MIRROR).isEmpty()) mirrorPending = true;
    }

    /**
     * GUI client rendering (UI_PROTOCOL §17) for the holder: a client that draws
     * scenes gets the phone's GUI as the scene stream (it fills the same display
     * cache entry the in-hand and open screens read). True if the holder got
     * the scene, so no tile pass is needed; the mirror for onlookers stays tiles.
     */
    private boolean sceneSync(UUID h) {
        ComputerInstance c = instance();
        if (c == null) return false;
        var ui = c.getUiManager();
        List<UUID> scene = ui.sceneViewers(DISPLAY_OWN, List.of(h));
        try {
            ui.sceneSync(DISPLAY_OWN, scene, (player, d, data) -> {
                ServerPlayer p = viewerSnapshot.player(player);
                if (p != null && !(p instanceof FakePlayer)) {
                    PacketDistributor.sendToPlayer(p, com.example.evanscomputermod.network.UiScenePackets.packet(player, id, d, data));
                }
            });
        } catch (Exception e) {
            EvansComputerMod.LOGGER.debug("ePhone scene sync failed", e);
        }
        if (scene.isEmpty()) return false;
        holderSync.retainViewers(List.of());
        return true;
    }

    @Override
    public void onSceneAck(ServerPlayer player, int displayId, int epoch, long generation, long[] missing) {
        onSceneAck(player.getUUID(), displayId, epoch, generation, missing, 0);
    }

    @Override
    public void onSceneAck(UUID player, int displayId, int epoch, long generation, long[] missing, long clientRecvUs) {
        ComputerInstance c = instance();
        if (c == null || !player.equals(holderId)) return;
        if (c.getUiManager().sceneHub().onAck(player, displayId, epoch, generation, missing, clientRecvUs)) syncToClients();
    }

    /** The phone's speaker: what programs wrote, to players near the holder. */
    private void playSpeaker() {
        short[] pcm = speakerAudio.drain();
        ServerPlayer h = holder;
        if (pcm.length == 0 || h == null) return;
        // The headphones layer: with AirPods connected and worn, the sound goes to them over
        // Bluetooth (only their wearer hears it; it isn't in the air for microphones either).
        if (bluetooth.audioRouted()) {
            bluetooth.link().sendAudio(pcm, speakerAudio.outputRate());
            return;
        }
        if (com.example.evanscomputermod.voice.VoiceServer.anyMic()) {
            // The phone's speaker is in the air for microphones too (in the holder's hand).
            var eye = h.getEyePosition();
            com.example.evanscomputermod.voice.SoundField.get().emit("phone:" + id, h.level().dimension().location().toString(),
                    eye.x, eye.y - 0.5, eye.z, pcm, 0, pcm.length, speakerAudio.outputRate(), 1f);
        }
        double range = com.example.evanscomputermod.EcmConfig.speakerRange();
        List<ServerPlayer> near = PhoneServer.playersNear(h.serverLevel(), h.position(), range);
        BlockPos at = h.blockPosition();
        // When the first sample plays (the card's queue ahead of it), for lip sync on the client (Part E2).
        int rate = Math.max(1, speakerAudio.outputRate());
        long now = com.example.evanscomputermod.computer.pipeline.DisplayClock.nowUs();
        boolean live = speakerAudio.lastDrainLive(); // a call plays at once, not on the picture's delay
        long pts = live ? now : Math.max(now, speakerAudio.lastDrainPtsUs()); // stamped by the mixer, after mixing and ducking
        int flags = live ? com.example.evanscomputermod.speaker.SpeakerAudioPacket.FLAG_LIVE : 0;
        for (int off = 0; off < pcm.length; off += 2600) {
            int n = Math.min(2600, pcm.length - off);
            if (near.isEmpty()) continue;
            var packet = new com.example.evanscomputermod.speaker.SpeakerAudioPacket(at, speakerSeq++, speakerAudio.outputRate(), n,
                    speakerEncoder.encode(pcm, off, n), live ? now : pts + off * 1_000_000L / rate, id, flags);
            for (ServerPlayer p : near) if (!(p instanceof FakePlayer)) PacketDistributor.sendToPlayer(p, packet);
        }
    }

    /** The phone's controller receiver (ControllerServer). */
    public com.example.evanscomputermod.controller.WirelessControllerHub controllers() {
        return controllers;
    }

    /** The phone's microphone. */
    public com.example.evanscomputermod.voice.MicAudio micAudio() {
        return micAudio;
    }

    /** The phone's speaker (tests read how much it played). */
    public com.example.evanscomputermod.speaker.SpeakerAudio speakerAudio() {
        return speakerAudio;
    }

    Object statusSourceFor() {
        return statusSource;
    }

    void statusSourceSet(Object ui) {
        statusSource = ui;
    }

    private void send(UUID player, TerminalDeltaPacket packet) {
        packetsTo.merge(player, 1, Integer::sum);
        ServerPlayer p = viewerSnapshot.player(player);
        if (p != null && !(p instanceof FakePlayer) && !p.hasDisconnected()) PacketDistributor.sendToPlayer(p, packet);
    }

    @Override
    public void forceNextKeyframe() {
        holderSync.forceKeyframe();
        mirrorSync.forceKeyframe();
    }

    @Override
    public IFramebufferDisplay getFramebufferDisplay() {
        return display;
    }

    /** The phone's display (server copy). */
    public TerminalDisplay display() {
        return display;
    }

    @Override
    public IRedstoneProvider getRedstoneProvider() {
        return null;
    }

    @Override
    public IWorldAccess getWorldAccess() {
        return null;
    }

    @Override
    public PeripheralHub getPeripheralHub() {
        return peripherals;
    }

    @Override
    @Nullable
    public PanelTier displayTier(int displayId) {
        return displayId == DISPLAY_OWN ? hardware.tier() : null;
    }

    @Override
    public PanelTier.Tone displayTone(int displayId) {
        return hardware.tone();
    }

    @Override
    public boolean portraitDisplay() {
        return true;
    }

    @Override
    public boolean acceptsPointerInput() {
        return true;
    }

    /** Holder: its own display. Onlookers in the mirror set: the mirror (acks only, see {@link #acceptsInput}). */
    @Override
    public boolean canInteract(ServerPlayer player, int displayId) {
        if (!attached) return false;
        if (displayId == DISPLAY_OWN) return player.getUUID().equals(holderId);
        if (displayId == DISPLAY_MIRROR) return mirrorViewers.contains(player.getUUID());
        return false;
    }

    /** Only the holder types and touches, on its own display, while the phone is awake. */
    @Override
    public boolean acceptsInput(ServerPlayer player, int displayId) {
        return displayId == DISPLAY_OWN && state == PhoneState.AWAKE && player.getUUID().equals(holderId);
    }

    @Override
    public void onDisplayAck(ServerPlayer player, int displayId, long generation) {
        onDisplayAck(player.getUUID(), displayId, generation, 0);
    }

    /** Any thread (acks come in on the network thread, Part A3). */
    @Override
    public void onDisplayAck(UUID player, int displayId, long generation, long clientRecvUs) {
        DisplaySyncHub hub = displayId == DISPLAY_MIRROR ? mirrorSync : holderSync;
        Runnable pass = displayId == DISPLAY_MIRROR ? mirrorLane::request : syncLane::request;
        // A holder whose client draws the GUI scene gets a scene keyframe instead.
        ComputerInstance ci = instance();
        if (generation == 0 && displayId == DISPLAY_OWN && ci != null
                && !ci.getUiManager().sceneViewers(displayId, List.of(player)).isEmpty()) {
            ci.getUiManager().sceneHub().requestKeyframe(player, displayId);
            syncToClients();
            return;
        }
        if (generation != 0) {
            if (hub.onClientReady(player, generation, clientRecvUs)) pass.run();
            return;
        }
        hub.requestKeyframe(player);
        pass.run();
    }

    // ------------------------------------------------------------ hotkeys and wakes

    /** In-hand hotkey from the holder (slot 1..6; 0 = switch app). */
    void hotkey(int slot, boolean down) {
        PhoneHotkeySink sink = PhoneHotkeys.sinkFor(this);
        if (slot == 0) {
            if (down) sink.switchApp(this);
        } else if (slot >= 1 && slot <= 6) {
            sink.slot(this, slot, down);
        }
    }

    /**
     * Background refresh (P4 scheduler): run the sleeping computer for up to
     * {@code budgetMs}, then sleep again. Null unless the phone is asleep and booted.
     */
    @Nullable
    public ComputerInstance.BudgetedWake refreshWake(long budgetMs) {
        ComputerInstance c = instance();
        if (state != PhoneState.ASLEEP || c == null) return null;
        return c.wakeFor(budgetMs);
    }

    // ------------------------------------------------------------ queries

    @Nullable
    public ComputerInstance instance() {
        return ComputerHost.get(server, id).instance();
    }

    public PhoneState state() {
        return state;
    }

    public boolean isAttached() {
        return attached;
    }

    public boolean isBooting() {
        return booting;
    }

    @Nullable
    public ServerPlayer holder() {
        return holder;
    }

    public String holderName() {
        return holderName;
    }

    @Nullable
    public UUID holderId() {
        return holderId;
    }

    public ModuleBays bays() {
        return bays;
    }

    /** Where the phone's antennas are: the holder's eye, a little lower (the phone in front of the face). */
    public Pose pose() {
        ServerPlayer p = holder;
        if (p == null) return Pose.at("minecraft:overworld", 0, -10_000, 0);
        Vec3 eye = p.getEyePosition();
        return Pose.at(p.level().dimension().location().toString(), eye.x, eye.y - 0.3, eye.z);
    }

    /** Display packets sent to {@code player} so far (tests). */
    public int packetsSentTo(UUID player) {
        return packetsTo.getOrDefault(player, 0);
    }

    public Set<UUID> mirrorViewers() {
        return Set.copyOf(mirrorViewers);
    }

    /** State transitions so far, e.g. {@code off>awake} (tests, debug). */
    public List<String> transitions() {
        return List.copyOf(transitions);
    }

    @Override
    public String toString() {
        return "ePhone " + id + " " + state + (attached ? " attached" : "") + " holder=" + holderName
                + String.format(" battery=%.1f%%", batteryPercent());
    }
}
//?}
