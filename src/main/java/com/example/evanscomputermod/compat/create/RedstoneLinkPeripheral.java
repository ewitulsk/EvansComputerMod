package com.example.evanscomputermod.compat.create;

//? if <=1.21.1 {
import com.example.evanscomputermod.api.peripheral.AnnotatedPeripheral;
import com.example.evanscomputermod.api.peripheral.IComputerAccess;
import com.example.evanscomputermod.api.peripheral.PeripheralException;
import com.example.evanscomputermod.api.peripheral.PeripheralMethod;
import com.simibubi.create.Create;
import com.simibubi.create.content.redstone.link.IRedstoneLinkable;
import com.simibubi.create.content.redstone.link.RedstoneLinkNetworkHandler;
import com.simibubi.create.content.redstone.link.RedstoneLinkNetworkHandler.Frequency;
import net.createmod.catnip.data.Couple;
import net.minecraft.core.BlockPos;
import net.minecraft.core.component.DataComponents;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.nbt.CompoundTag;
import net.minecraft.nbt.ListTag;
import net.minecraft.nbt.Tag;
import net.minecraft.resources.Identifier;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.world.item.Item;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.item.Items;
import net.minecraft.world.item.component.DyedItemColor;
import org.jetbrains.annotations.Nullable;

import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.Set;
import java.util.concurrent.CopyOnWriteArraySet;

/**
 * A 64-channel Create Redstone Link transceiver. Each channel has a frequency
 * pair (the two item slots of a Create Redstone Link, e.g.
 * {@code ("minecraft:red_dye", "minecraft:lapis_lazuli")}) and a mode:
 * {@code "tx"} transmits the channel's output strength, {@code "rx"} receives
 * the strongest transmitter in range, {@code "off"} does nothing.
 *
 * <p>Each active channel joins Create's link network as its own
 * {@link IRedstoneLinkable} at the computer's position, so it talks to real
 * Create Redstone Links, Linked Controllers and other computers. On a Sable /
 * Create Aeronautics structure, range is measured in world space.
 *
 * <p>Used by the Redstone Link module (in a computer's bay) and the Redstone
 * Link Interface block (next to a computer). All methods run on the server thread.
 */
public class RedstoneLinkPeripheral extends AnnotatedPeripheral {

    public static final String TYPE = "redstone_link";
    public static final int CHANNELS = 64;
    /** How often received strengths are re-checked and transmitters re-announced while on a moving structure. */
    private static final int RESYNC_TICKS = 10;

    public enum Mode { OFF, TX, RX }

    /** Where the peripheral is and whether it's in the world. */
    public interface Location {
        @Nullable ServerLevel level();

        BlockPos pos();

        /** Configuration changed (persist it). */
        void markDirty();
    }

    private static final class Channel {
        ItemStack first = ItemStack.EMPTY;
        ItemStack second = ItemStack.EMPTY;
        Mode mode = Mode.OFF;
        int output;
        int input;
        int reported;
        @Nullable Link link;

        boolean isDefault() {
            return first.isEmpty() && second.isEmpty() && mode == Mode.OFF && output == 0;
        }
    }

    /** One channel's membership in a Create link network. */
    private final class Link implements IRedstoneLinkable {
        final int index;
        final boolean listening;
        final Couple<Frequency> key;

        Link(int index, Channel c) {
            this.index = index;
            this.listening = c.mode == Mode.RX;
            this.key = Couple.create(Frequency.of(c.first), Frequency.of(c.second));
        }

        @Override
        public int getTransmittedStrength() {
            Channel c = channels[index];
            return !listening && c.link == this ? c.output : 0;
        }

        @Override
        public void setReceivedStrength(int power) {
            Channel c = channels[index];
            if (listening && c.link == this) received(index, power);
        }

        @Override
        public boolean isListening() {
            return listening;
        }

        @Override
        public boolean isAlive() {
            return live && channels[index].link == this;
        }

        @Override
        public Couple<Frequency> getNetworkKey() {
            return key;
        }

        @Override
        public BlockPos getLocation() {
            return location.pos();
        }
    }

    private final Location location;
    private final Channel[] channels = new Channel[CHANNELS];
    private final Set<IComputerAccess> computers = new CopyOnWriteArraySet<>();
    private boolean live;
    private boolean pendingEvents;
    private int ticks;

    public RedstoneLinkPeripheral(Location location) {
        this.location = location;
        for (int i = 0; i < CHANNELS; i++) channels[i] = new Channel();
    }

    @Override
    public String getType() {
        return TYPE;
    }

    @Override
    public void attach(IComputerAccess computer) {
        computers.add(computer);
    }

    @Override
    public void detach(IComputerAccess computer) {
        computers.remove(computer);
    }

    // ------------------------------------------------------------ lifecycle (server thread)

    /** Join the link networks for every active channel. */
    public void load() {
        if (live || location.level() == null) return;
        live = true;
        for (int i = 0; i < CHANNELS; i++) connect(i);
    }

    /** Leave all link networks (removed, unloaded, or about to move). */
    public void unload() {
        if (!live) return;
        for (int i = 0; i < CHANNELS; i++) disconnect(i);
        live = false;
    }

    public void tick() {
        if (!live) return;
        ServerLevel level = location.level();
        if (level != null && ++ticks >= RESYNC_TICKS && LinkRange.isOnSubLevel(level, location.pos())) {
            // Create only re-evaluates a network when a member changes, so a
            // moving structure would keep stale strengths. Re-check ours and
            // re-announce our transmitters.
            ticks = 0;
            for (int i = 0; i < CHANNELS; i++) {
                Link link = channels[i].link;
                if (link == null) continue;
                if (link.listening) received(i, strongestInRange(level, link));
                else if (channels[i].output > 0) handler().updateNetworkOf(level, link);
            }
        }
        if (pendingEvents) {
            pendingEvents = false;
            for (int i = 0; i < CHANNELS; i++) {
                Channel c = channels[i];
                if (c.input != c.reported) {
                    int old = c.reported;
                    c.reported = c.input;
                    for (IComputerAccess computer : computers) {
                        computer.queueEvent("redstone_link", i, c.input, old);
                    }
                }
            }
        }
    }

    private static RedstoneLinkNetworkHandler handler() {
        return Create.REDSTONE_LINK_NETWORK_HANDLER;
    }

    private void connect(int index) {
        Channel c = channels[index];
        ServerLevel level = location.level();
        if (!live || level == null || c.mode == Mode.OFF || c.link != null) return;
        Link link = new Link(index, c);
        c.link = link;
        handler().addToNetwork(level, link);
        if (link.listening) {
            // A joining member that isn't Create's own LinkBehaviour isn't told the
            // current strength; work it out.
            received(index, strongestInRange(level, link));
        }
    }

    private void disconnect(int index) {
        Channel c = channels[index];
        Link link = c.link;
        if (link == null) return;
        c.link = null;
        ServerLevel level = location.level();
        if (level != null) handler().removeFromNetwork(level, link);
        if (link.listening) received(index, 0);
    }

    private void reconnect(int index) {
        disconnect(index);
        connect(index);
    }

    private int strongestInRange(ServerLevel level, Link self) {
        int power = 0;
        for (IRedstoneLinkable other : handler().getNetworkOf(level, self)) {
            if (other == self || !other.isAlive() || other.isListening()) continue;
            if (LinkRange.withinRange(level, self.getLocation(), other.getLocation())) {
                power = Math.max(power, other.getTransmittedStrength());
                if (power >= 15) break;
            }
        }
        return power;
    }

    private void received(int index, int power) {
        Channel c = channels[index];
        power = Math.max(0, Math.min(15, power));
        if (c.input != power) {
            c.input = power;
            pendingEvents = true;
        }
    }

    // ------------------------------------------------------------ persistence

    public void save(CompoundTag tag) {
        ListTag list = new ListTag();
        for (int i = 0; i < CHANNELS; i++) {
            Channel c = channels[i];
            if (c.isDefault()) continue;
            CompoundTag ct = new CompoundTag();
            ct.putInt("channel", i);
            ct.putString("mode", c.mode.name().toLowerCase(Locale.ROOT));
            ct.putInt("output", c.output);
            putFrequency(ct, "first", c.first);
            putFrequency(ct, "second", c.second);
            list.add(ct);
        }
        tag.put("channels", list);
    }

    public void load(CompoundTag tag) {
        for (int i = 0; i < CHANNELS; i++) {
            disconnect(i);
            channels[i] = new Channel();
        }
        ListTag list = tag.getList("channels", Tag.TAG_COMPOUND);
        for (int i = 0; i < list.size(); i++) {
            CompoundTag ct = list.getCompound(i);
            int index = ct.getInt("channel");
            if (index < 0 || index >= CHANNELS) continue;
            Channel c = channels[index];
            c.mode = parseMode(ct.getString("mode"), Mode.OFF);
            c.output = Math.max(0, Math.min(15, ct.getInt("output")));
            c.first = getFrequency(ct, "first");
            c.second = getFrequency(ct, "second");
            connect(index);
        }
    }

    private static void putFrequency(CompoundTag ct, String key, ItemStack stack) {
        if (stack.isEmpty()) return;
        ct.putString(key, BuiltInRegistries.ITEM.getKey(stack.getItem()).toString());
        DyedItemColor dye = stack.get(DataComponents.DYED_COLOR);
        if (dye != null) ct.putInt(key + "_color", dye.rgb());
    }

    private static ItemStack getFrequency(CompoundTag ct, String key) {
        if (!ct.contains(key)) return ItemStack.EMPTY;
        Identifier id = Identifier.tryParse(ct.getString(key));
        Item item = id == null ? null : BuiltInRegistries.ITEM.getOptional(id).orElse(null);
        if (item == null || item == Items.AIR) return ItemStack.EMPTY;
        ItemStack stack = new ItemStack(item);
        if (ct.contains(key + "_color")) stack.set(DataComponents.DYED_COLOR, new DyedItemColor(ct.getInt(key + "_color"), false));
        return stack;
    }

    // ------------------------------------------------------------ program API

    @PeripheralMethod(description = "Number of channels (64)", mainThread = false)
    public int getChannelCount() {
        return CHANNELS;
    }

    @PeripheralMethod(description = "Configure a channel: two frequency items and a mode (tx / rx / off)")
    public void setChannel(int channel, @Nullable Object first, @Nullable Object second, String mode) throws PeripheralException {
        Channel c = channel(channel);
        Mode m = mode(mode);
        c.first = frequency(first, "first");
        c.second = frequency(second, "second");
        c.mode = m;
        reconnect(channel);
        location.markDirty();
    }

    @PeripheralMethod(description = "Set a channel's two frequency items, keeping its mode")
    public void setFrequency(int channel, @Nullable Object first, @Nullable Object second) throws PeripheralException {
        Channel c = channel(channel);
        c.first = frequency(first, "first");
        c.second = frequency(second, "second");
        reconnect(channel);
        location.markDirty();
    }

    @PeripheralMethod(description = "Set a channel's mode: tx, rx or off")
    public void setMode(int channel, String mode) throws PeripheralException {
        Channel c = channel(channel);
        Mode m = mode(mode);
        if (c.mode == m) return;
        c.mode = m;
        reconnect(channel);
        location.markDirty();
    }

    @PeripheralMethod(description = "Set the strength (0-15) a tx channel transmits")
    public void setOutput(int channel, int power) throws PeripheralException {
        Channel c = channel(channel);
        setOutputOf(channel, c, power);
    }

    @PeripheralMethod(description = "Set several tx strengths at once: {channel: power}")
    public void setOutputs(Map<?, ?> powers) throws PeripheralException {
        List<int[]> changes = new ArrayList<>();
        for (Map.Entry<?, ?> e : powers.entrySet()) {
            if (!(e.getKey() instanceof Number ch) || !(e.getValue() instanceof Number p)) {
                throw new PeripheralException("set_outputs expects {channel: power} with integer keys and values");
            }
            channel(ch.intValue());
            changes.add(new int[]{ch.intValue(), power(p.intValue())});
        }
        for (int[] change : changes) setOutputOf(change[0], channels[change[0]], change[1]);
    }

    private void setOutputOf(int index, Channel c, int power) throws PeripheralException {
        int p = power(power);
        if (c.output == p) return;
        c.output = p;
        ServerLevel level = location.level();
        if (c.link != null && !c.link.listening && level != null) {
            handler().updateNetworkOf(level, c.link);
        }
        location.markDirty();
    }

    @PeripheralMethod(description = "The strength a tx channel transmits")
    public int getOutput(int channel) throws PeripheralException {
        return channel(channel).output;
    }

    @PeripheralMethod(description = "The strength an rx channel receives (0 when not rx)")
    public int getInput(int channel) throws PeripheralException {
        Channel c = channel(channel);
        return c.mode == Mode.RX ? c.input : 0;
    }

    @PeripheralMethod(description = "Received strength of every channel, as a list of 64")
    public List<Integer> getInputs() {
        List<Integer> out = new ArrayList<>(CHANNELS);
        for (Channel c : channels) out.add(c.mode == Mode.RX ? c.input : 0);
        return out;
    }

    @PeripheralMethod(description = "A channel's configuration and strengths")
    public Map<String, Object> getChannel(int channel) throws PeripheralException {
        return describe(channel, channel(channel));
    }

    @PeripheralMethod(description = "Every configured channel")
    public List<Map<String, Object>> getChannels() {
        List<Map<String, Object>> out = new ArrayList<>();
        for (int i = 0; i < CHANNELS; i++) {
            if (!channels[i].isDefault()) out.add(describe(i, channels[i]));
        }
        return out;
    }

    @PeripheralMethod(description = "Turn a channel off and clear its frequency")
    public void clear(int channel) throws PeripheralException {
        channel(channel);
        disconnect(channel);
        channels[channel] = new Channel();
        location.markDirty();
    }

    @PeripheralMethod(description = "Turn every channel off and clear all frequencies")
    public void clearAll() {
        for (int i = 0; i < CHANNELS; i++) {
            disconnect(i);
            channels[i] = new Channel();
        }
        location.markDirty();
    }

    private Map<String, Object> describe(int index, Channel c) {
        Map<String, Object> m = new LinkedHashMap<>();
        m.put("channel", index);
        m.put("first", describe(c.first));
        m.put("second", describe(c.second));
        m.put("mode", c.mode);
        m.put("output", c.output);
        m.put("input", c.mode == Mode.RX ? c.input : 0);
        return m;
    }

    @Nullable
    private static Object describe(ItemStack stack) {
        if (stack.isEmpty()) return null;
        String id = BuiltInRegistries.ITEM.getKey(stack.getItem()).toString();
        DyedItemColor dye = stack.get(DataComponents.DYED_COLOR);
        if (dye == null) return id;
        Map<String, Object> m = new LinkedHashMap<>();
        m.put("item", id);
        m.put("color", dye.rgb());
        return m;
    }

    // ------------------------------------------------------------ argument checks

    private Channel channel(int channel) throws PeripheralException {
        if (channel < 0 || channel >= CHANNELS) {
            throw new PeripheralException("channel must be 0-" + (CHANNELS - 1) + ", got " + channel);
        }
        return channels[channel];
    }

    private static int power(int power) throws PeripheralException {
        if (power < 0 || power > 15) throw new PeripheralException("power must be 0-15, got " + power);
        return power;
    }

    private static Mode mode(@Nullable String mode) throws PeripheralException {
        Mode m = mode == null ? null : parseMode(mode, null);
        if (m == null) throw new PeripheralException("mode must be \"tx\", \"rx\" or \"off\", got " + mode);
        return m;
    }

    @Nullable
    private static Mode parseMode(String s, @Nullable Mode fallback) {
        return switch (s.toLowerCase(Locale.ROOT)) {
            case "tx", "transmit", "send" -> Mode.TX;
            case "rx", "receive" -> Mode.RX;
            case "off" -> Mode.OFF;
            default -> fallback;
        };
    }

    /**
     * A frequency item from a program value: an item id string
     * ({@code "minecraft:red_dye"}), {@code None} / {@code ""} for an empty
     * slot, or {@code {"item": id, "color": rgb}} for a dyed item (Create
     * tells dyed items apart by color).
     */
    private static ItemStack frequency(@Nullable Object value, String which) throws PeripheralException {
        if (value == null) return ItemStack.EMPTY;
        String id;
        Integer color = null;
        if (value instanceof String s) {
            id = s;
        } else if (value instanceof Map<?, ?> m && m.get("item") instanceof String s) {
            id = s;
            Object c = m.get("color");
            if (c instanceof Number n) color = n.intValue();
            else if (c != null) throw new PeripheralException(which + " frequency color must be an integer RGB value");
        } else {
            throw new PeripheralException(which + " frequency must be an item id like \"minecraft:red_dye\", "
                    + "{\"item\": id, \"color\": rgb} or None");
        }
        if (id.isEmpty() || id.equals("minecraft:air")) return ItemStack.EMPTY;
        Identifier rl = Identifier.tryParse(id);
        Item item = rl == null ? null : BuiltInRegistries.ITEM.getOptional(rl).orElse(null);
        if (item == null || item == Items.AIR) {
            throw new PeripheralException("unknown item '" + id + "' for " + which + " frequency");
        }
        ItemStack stack = new ItemStack(item);
        if (color != null) stack.set(DataComponents.DYED_COLOR, new DyedItemColor(color & 0xFFFFFF, false));
        return stack;
    }
}
//?}
