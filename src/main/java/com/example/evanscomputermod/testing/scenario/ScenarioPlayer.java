package com.example.evanscomputermod.testing.scenario;

//? if <=1.21.1 {
import com.example.evanscomputermod.block.ModBlocks;
import com.example.evanscomputermod.block.TerminalBlock;
import com.example.evanscomputermod.block.TerminalMenu;
import com.example.evanscomputermod.item.ModItems;
import com.example.evanscomputermod.network.KeyInputPacket;
import com.example.evanscomputermod.network.MouseInputPacket;
import com.example.evanscomputermod.network.TerminalInputPacket;
import com.example.evanscomputermod.module.ModuleBays;
import com.example.evanscomputermod.radio.wifi.ap.AccessPointBlockEntity;
import com.example.evanscomputermod.radio.wifi.ap.ApPackets;
import com.example.evanscomputermod.radio.wifi.ap.ApSettings;
import com.mojang.authlib.GameProfile;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.network.chat.Component;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.world.InteractionHand;
import net.minecraft.world.InteractionResult;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.level.GameType;
import net.minecraft.world.level.ItemLike;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.phys.BlockHitResult;
import net.minecraft.world.phys.Vec3;
import net.neoforged.neoforge.common.util.FakePlayer;

import java.util.ArrayList;
import java.util.List;
import java.util.UUID;
import java.util.concurrent.CopyOnWriteArrayList;
import java.util.function.Predicate;

/**
 * The hands of a scenario: a creative-mode player that builds and operates a
 * scenario the way a person at the keyboard would. It stands next to a block,
 * looks at it and right-clicks through the game's own interaction code
 * ({@code ServerPlayerGameMode.useItemOn / useItem / destroyBlock}), so block
 * placement runs {@code getStateForPlacement}/{@code setPlacedBy} and the place
 * event, items run their {@code useOn}, and blocks their
 * {@code useItemOn}/{@code useWithoutItem}. Whatever the game tells the player
 * (chat or action bar) is recorded, so a scenario can check what a person
 * would have read. Commands run as this player with operator rights (cheats on).
 *
 * <p>Scenarios built with it must not reach into block entities to change
 * them: everything they change goes through this class, a computer's
 * keyboard, or a command the player could type. Screens open for real (the
 * block's menu becomes the player's open container; NeoForge's FakePlayer would
 * refuse it), and their buttons (the Access Point's Apply) run the same
 * server-side handler their packets call, with this player as the sender. Keys,
 * text and clicks go through the input packets' server handlers.
 */
public final class ScenarioPlayer {
    public static final String NAME = "ScenarioBuilder";
    private static final UUID ID = UUID.nameUUIDFromBytes(("ecm-scenario:" + NAME).getBytes());

    private final ServerLevel level;
    private final Hands hands;

    /** A fake player that keeps what the game says to it (messages may arrive from solver threads). */
    private static final class Hands extends FakePlayer {
        final List<String> heard = new CopyOnWriteArrayList<>();

        Hands(ServerLevel level, GameProfile profile) {
            super(level, profile);
        }

        private int menuCounter;

        /**
         * NeoForge's FakePlayer refuses every screen. A scenario player opens them for
         * real: the block's menu is created and becomes this player's open container,
         * as on a server for a connected player (the open-screen packet has nowhere to
         * go), so screens' range and "is this menu open" checks see what a client's
         * player would give them.
         */
        @Override
        public java.util.OptionalInt openMenu(net.minecraft.world.MenuProvider provider,
                                              java.util.function.Consumer<net.minecraft.network.RegistryFriendlyByteBuf> extraData) {
            if (provider == null) return java.util.OptionalInt.empty();
            if (containerMenu != inventoryMenu) closeContainer();
            menuCounter = menuCounter % 100 + 1;
            net.minecraft.world.inventory.AbstractContainerMenu menu = provider.createMenu(menuCounter, getInventory(), this);
            if (menu == null) return java.util.OptionalInt.empty();
            containerMenu = menu;
            net.neoforged.neoforge.common.NeoForge.EVENT_BUS.post(new net.neoforged.neoforge.event.entity.player.PlayerContainerEvent.Open(this, menu));
            return java.util.OptionalInt.of(menuCounter);
        }

        @Override
        public java.util.OptionalInt openMenu(net.minecraft.world.MenuProvider provider) {
            return openMenu(provider, (java.util.function.Consumer<net.minecraft.network.RegistryFriendlyByteBuf>) null);
        }

        @Override
        public void displayClientMessage(Component message, boolean actionBar) {
            heard.add(message.getString());
        }

        @Override
        public void sendSystemMessage(Component message, boolean overlay) {
            heard.add(message.getString());
        }

        @Override
        public void sendSystemMessage(Component message) {
            heard.add(message.getString());
        }
    }

    public ScenarioPlayer(ServerLevel level) {
        this(level, NAME);
    }

    /** A second (third...) player with its own name and id, for scenarios with more than one person (the ePhone's onlookers). */
    public ScenarioPlayer(ServerLevel level, String name) {
        this.level = level;
        UUID id = name.equals(NAME) ? ID : UUID.nameUUIDFromBytes(("ecm-scenario:" + name).getBytes());
        this.hands = new Hands(level, new GameProfile(id, name));
        hands.setGameMode(GameType.CREATIVE);
    }

    public FakePlayer entity() {
        return hands;
    }

    /** Everything the game said to the player since the last {@link #clearHeard}. */
    public List<String> heard() {
        return List.copyOf(hands.heard);
    }

    public void clearHeard() {
        hands.heard.clear();
    }

    /** The last thing the game said, or "". */
    public String lastHeard() {
        List<String> h = heard();
        return h.isEmpty() ? "" : h.get(h.size() - 1);
    }

    // ------------------------------------------------------------ building a scenario's layout

    /** Places the scenario's terminals (screens facing their {@code facing}) and cable runs, one right-click each. */
    void buildLayout(ScenarioRun run) {
        for (Scenario.Node n : run.scenario().nodes.values()) {
            Direction f = n.facing();
            place(ModBlocks.TERMINAL_BLOCK.get(), run.abs(n.pos()), f, s -> s.getValue(TerminalBlock.FACING) == f);
        }
        for (Scenario.Link l : run.scenario().links)
            for (BlockPos p : l.cable()) place(ModBlocks.NETWORK_CABLE.get(), run.abs(p), Direction.UP);
    }

    /** Right-click a terminal's screen: opens it, which boots the computer. */
    public void openScreen(BlockPos terminal, Direction screen) {
        useEmpty(terminal, screen, false);
        hands.closeContainer();
    }

    /**
     * Click a Module Expansion Card onto the computer's left side (opening the
     * bay there unless it is open already), then {@code module} onto the upper
     * half of that side. Returns what the game said (action-bar keys).
     */
    public List<String> installModule(BlockPos terminal, ItemLike module) {
        BlockState s = level.getBlockState(terminal);
        Direction facing = s.getValue(TerminalBlock.FACING);
        Direction left = null;
        for (Direction d : Direction.Plane.HORIZONTAL) if (ModuleBays.bayOnFace(facing, d) == ModuleBays.LEFT) left = d;
        if (left == null) throw new IllegalStateException("no left bay face on " + s);
        List<String> said = new ArrayList<>();
        if (!s.getValue(TerminalBlock.LEFT_BAY)) said.addAll(use(new ItemStack(ModItems.MODULE_EXPANSION_CARD.get()), terminal, left, false));
        Vec3 upper = faceCentre(terminal, left).add(0, 0.25, 0);
        said.addAll(use(new ItemStack(module), terminal, left, upper, false));
        return said;
    }

    /**
     * What a player does in the Access Point screen: right-click the AP (opening
     * it), fill in the settings and press Apply. The server runs the same
     * handler as the screen's Configure packet; returns its reply ("Settings
     * applied", or why not).
     */
    public String configureAccessPoint(BlockPos ap, ApSettings settings, String passphrase) {
        useEmpty(ap, Direction.UP, false);
        if (hands.containerMenu == hands.inventoryMenu)
            throw new IllegalStateException("right-clicking the access point at " + ap + " opened no screen");
        // Apply: the Configure packet's server handler, sent while the screen is open; then Escape.
        String reply = ApPackets.configure(hands, new ApPackets.Configure(ap, ApPackets.Action.APPLY, settings,
                passphrase == null ? "" : passphrase, 0));
        hands.closeContainer();
        hands.heard.add(reply);
        return reply;
    }

    /**
     * Right-click {@code face} of {@code pos} to open its screen: the block's own use code
     * opens its menu through this player's {@code openMenu} (which opens it for real, see
     * the player above). Returns the open menu, or null if the block opened none.
     */
    public net.minecraft.world.inventory.AbstractContainerMenu openMenu(BlockPos pos, Direction face, boolean sneaking) {
        use(ItemStack.EMPTY, pos, face, sneaking);
        return hands.containerMenu != hands.inventoryMenu ? hands.containerMenu : null;
    }

    /**
     * Press button {@code id} of the screen open now: what the client's
     * {@code handleInventoryButtonClick} sends, run by the same steps as the server's
     * handler (ServerGamePacketListenerImpl.handleContainerButtonClick). False if no
     * screen is open, it is out of reach, or the button did nothing.
     */
    public boolean pressMenuButton(int id) {
        var menu = hands.containerMenu;
        if (menu == hands.inventoryMenu || !menu.stillValid(hands)) return false;
        boolean done = menu.clickMenuButton(hands, id);
        if (done) menu.broadcastChanges();
        return done;
    }

    /**
     * Shift-click slot {@code slot} of the screen open now (a chest: take that stack into
     * the inventory), as the container-click packet does. Returns what moved.
     */
    public ItemStack shiftClick(int slot) {
        var menu = hands.containerMenu;
        ItemStack before = menu.getSlot(slot).getItem().copy();
        menu.clicked(slot, 0, net.minecraft.world.inventory.ClickType.QUICK_MOVE, hands);
        menu.broadcastChanges();
        return before;
    }

    /** What the Access Point screen shows (the view its Status packet carries); null if there is no AP. */
    public ApPackets.ApView accessPointScreen(BlockPos ap) {
        return level.getBlockEntity(ap) instanceof AccessPointBlockEntity be ? be.view(null) : null;
    }

    // ------------------------------------------------------------ moving and looking

    /** Stand with the eyes at {@code eye}, looking at {@code target}. */
    public void lookFrom(Vec3 eye, Vec3 target) {
        double eyeHeight = hands.isShiftKeyDown() ? 1.27 : 1.62;
        Vec3 d = target.subtract(eye);
        float yaw = (float) (Math.toDegrees(Math.atan2(-d.x, d.z)));
        float pitch = (float) (-Math.toDegrees(Math.atan2(d.y, Math.sqrt(d.x * d.x + d.z * d.z))));
        hands.moveTo(eye.x, eye.y - eyeHeight, eye.z, yaw, pitch);
        hands.setYHeadRot(yaw);
    }

    private void sneak(boolean on) {
        hands.setShiftKeyDown(on);
    }

    // ------------------------------------------------------------ placing

    /**
     * Place {@code item} at {@code pos} like a player standing on the
     * {@code standOn} side of it, two blocks away, at the block's height and
     * looking at it (so blocks that face the player face {@code standOn}, and
     * blocks that face away from the player face {@code standOn.getOpposite()}).
     * The player clicks a face of an existing neighbour (sneaking, so an
     * interactive neighbour isn't opened instead); with no neighbour to
     * click, it props a dirt block underneath and breaks it afterwards, as
     * a player building in the air would.
     */
    public BlockState place(ItemLike item, BlockPos pos, Direction standOn) {
        return place(new ItemStack(item), pos, standOn, null, null);
    }

    /** {@link #place}, failing when the placed state doesn't satisfy {@code expect}. */
    public BlockState place(ItemLike item, BlockPos pos, Direction standOn, Predicate<BlockState> expect) {
        return place(new ItemStack(item), pos, standOn, expect, null);
    }

    /** {@link #place} clicking the given neighbour face: {@code against} is the side of {@code pos} the clicked block is on. */
    public BlockState placeAgainst(ItemLike item, BlockPos pos, Direction standOn, Direction against, Predicate<BlockState> expect) {
        return place(new ItemStack(item), pos, standOn, expect, against);
    }

    public BlockState place(ItemStack stack, BlockPos pos, Direction standOn, Predicate<BlockState> expect, Direction against) {
        if (!level.getBlockState(pos).canBeReplaced())
            throw new IllegalStateException("can't place " + stack.getHoverName().getString() + " at " + pos
                    + ": occupied by " + level.getBlockState(pos));
        BlockPos support = null;
        Direction clickFace = null;
        List<Direction> order = new ArrayList<>();
        if (against != null) order.add(against);
        else {
            // Prefer the block below, then the side away from the player, then any other.
            order.addAll(List.of(Direction.DOWN, standOn.getOpposite(), standOn));
            for (Direction d : Direction.values()) if (!order.contains(d)) order.add(d);
        }
        for (Direction d : order) {
            BlockPos n = pos.relative(d);
            BlockState s = level.getBlockState(n);
            if (!s.isAir() && !s.canBeReplaced() && s.getFluidState().isEmpty()) {
                support = n;
                clickFace = d.getOpposite();
                break;
            }
        }
        if (support == null && against != null) throw new IllegalStateException("nothing on the " + against + " of " + pos + " to click");
        boolean propped = false;
        if (support == null) {
            support = pos.below();
            if (!level.getBlockState(support).canBeReplaced())
                throw new IllegalStateException("nothing to click to place at " + pos);
            level.setBlock(support, Blocks.DIRT.defaultBlockState(), 3);
            clickFace = Direction.UP;
            propped = true;
        }
        Vec3 hit = Vec3.atCenterOf(support).add(Vec3.atLowerCornerOf(clickFace.getNormal()).scale(0.5));
        Vec3 eye = Vec3.atCenterOf(pos).add(Vec3.atLowerCornerOf(standOn.getNormal()).scale(2.0));
        sneak(true);
        lookFrom(eye, Vec3.atCenterOf(pos));
        hands.setItemInHand(InteractionHand.MAIN_HAND, stack.copy());
        InteractionResult r = hands.gameMode.useItemOn(hands, level, hands.getMainHandItem(), InteractionHand.MAIN_HAND,
                new BlockHitResult(hit, clickFace, support, false));
        sneak(false);
        hands.setItemInHand(InteractionHand.MAIN_HAND, ItemStack.EMPTY);
        if (propped) hands.gameMode.destroyBlock(support);
        BlockState placed = level.getBlockState(pos);
        if (!r.consumesAction() || placed.isAir() || placed.canBeReplaced())
            throw new IllegalStateException("placing " + stack.getHoverName().getString() + " at " + pos + " failed: " + r);
        if (expect != null && !expect.test(placed))
            throw new IllegalStateException("placed " + placed + " at " + pos + " (standing " + standOn + "), not what the scenario wanted");
        return placed;
    }

    // ------------------------------------------------------------ using

    /**
     * Right-click {@code face} of the block at {@code pos} holding {@code held}
     * (empty for a bare hand), aiming at {@code hit} (a point on that face),
     * optionally sneaking. Returns what the game said to the player.
     */
    public List<String> use(ItemStack held, BlockPos pos, Direction face, Vec3 hit, boolean sneaking) {
        clearHeard();
        Vec3 eye = hit.add(Vec3.atLowerCornerOf(face.getNormal()).scale(1.5));
        sneak(sneaking);
        lookFrom(eye, hit);
        hands.setItemInHand(InteractionHand.MAIN_HAND, held.copy());
        hands.gameMode.useItemOn(hands, level, hands.getMainHandItem(), InteractionHand.MAIN_HAND,
                new BlockHitResult(hit, face, pos, false));
        sneak(false);
        hands.setItemInHand(InteractionHand.MAIN_HAND, ItemStack.EMPTY);
        return heard();
    }

    /**
     * {@link #use(ItemStack, BlockPos, Direction, Vec3, boolean)} holding {@code held}
     * itself rather than a copy, so what the click writes into the stack (a
     * Sensor Wire's pending connection) stays on it for the next click.
     */
    public List<String> useHolding(ItemStack held, BlockPos pos, Direction face, Vec3 hit) {
        clearHeard();
        lookFrom(hit.add(Vec3.atLowerCornerOf(face.getNormal()).scale(1.5)), hit);
        hands.setItemInHand(InteractionHand.MAIN_HAND, held);
        hands.gameMode.useItemOn(hands, level, hands.getMainHandItem(), InteractionHand.MAIN_HAND,
                new BlockHitResult(hit, face, pos, false));
        hands.setItemInHand(InteractionHand.MAIN_HAND, ItemStack.EMPTY);
        return heard();
    }

    /** Right-click the middle of {@code face} of {@code pos}. */
    public List<String> use(ItemStack held, BlockPos pos, Direction face, boolean sneaking) {
        return use(held, pos, face, faceCentre(pos, face), sneaking);
    }

    public List<String> use(ItemLike held, BlockPos pos, Direction face, boolean sneaking) {
        return use(new ItemStack(held), pos, face, sneaking);
    }

    public List<String> useEmpty(BlockPos pos, Direction face, boolean sneaking) {
        return use(ItemStack.EMPTY, pos, face, sneaking);
    }

    /** Right-click the air holding {@code held} (standing where the player is), returning what the game said. */
    public List<String> useInAir(ItemStack held, boolean sneaking) {
        clearHeard();
        sneak(sneaking);
        hands.setItemInHand(InteractionHand.MAIN_HAND, held);
        hands.gameMode.useItem(hands, level, held, InteractionHand.MAIN_HAND);
        sneak(false);
        hands.setItemInHand(InteractionHand.MAIN_HAND, ItemStack.EMPTY);
        return heard();
    }

    public static Vec3 faceCentre(BlockPos pos, Direction face) {
        return Vec3.atCenterOf(pos).add(Vec3.atLowerCornerOf(face.getNormal()).scale(0.5));
    }

    /** Break the block at {@code pos} (creative: instantly, no drops). */
    public void breakBlock(BlockPos pos, Direction from) {
        lookFrom(Vec3.atCenterOf(pos).add(Vec3.atLowerCornerOf(from.getNormal()).scale(2.0)), Vec3.atCenterOf(pos));
        if (!hands.gameMode.destroyBlock(pos))
            throw new IllegalStateException("couldn't break " + level.getBlockState(pos) + " at " + pos);
    }

    /**
     * Break the block at {@code pos} in survival with a diamond pickaxe, so it
     * drops what a survival player gets (a computer drops itself with its id,
     * modules and panel). Back in creative afterwards.
     */
    public void breakForDrop(BlockPos pos) {
        hands.closeContainer();
        hands.setGameMode(GameType.SURVIVAL);
        try {
            lookFrom(Vec3.atCenterOf(pos).add(0, 0, 2.0), Vec3.atCenterOf(pos));
            hands.setItemInHand(InteractionHand.MAIN_HAND, new ItemStack(net.minecraft.world.item.Items.DIAMOND_PICKAXE));
            BlockState was = level.getBlockState(pos);
            if (!hands.gameMode.destroyBlock(pos))
                throw new IllegalStateException("couldn't break " + level.getBlockState(pos) + " at " + pos);
            if (level.getEntitiesOfClass(net.minecraft.world.entity.item.ItemEntity.class, new net.minecraft.world.phys.AABB(pos).inflate(2)).isEmpty())
                throw new IllegalStateException("breaking " + was + " dropped nothing (doTileDrops "
                        + level.getGameRules().getBoolean(net.minecraft.world.level.GameRules.RULE_DOBLOCKDROPS) + ", harvestable "
                        + was.canHarvestBlock(level, pos, hands) + ", now " + level.getBlockState(pos) + ", mode " + hands.gameMode.getGameModeForPlayer() + ")");
        } finally {
            hands.setItemInHand(InteractionHand.MAIN_HAND, ItemStack.EMPTY);
            hands.setGameMode(GameType.CREATIVE);
        }
    }

    /**
     * Walk to a dropped {@code item} within 4 blocks of {@code near} and pick it
     * up (the item entity's own player-touch pickup, so its pickup delay
     * applies). True once one is in the inventory.
     */
    public boolean pickUpNear(BlockPos near, ItemLike item) {
        return pickUp(near, item) == null;
    }

    /** {@link #pickUpNear}: null once picked up, else what the player sees (for a step's wait message). */
    public String pickUp(BlockPos near, ItemLike item) {
        StringBuilder seen = new StringBuilder();
        for (var e : level.getEntitiesOfClass(net.minecraft.world.entity.item.ItemEntity.class,
                new net.minecraft.world.phys.AABB(near).inflate(4))) {
            seen.append(' ').append(e.getItem()).append(e.hasPickUpDelay() ? " (pickup delay)" : "");
            if (!e.getItem().is(item.asItem())) continue;
            hands.moveTo(e.getX(), e.getY(), e.getZ(), hands.getYRot(), hands.getXRot());
            e.playerTouch(hands);
        }
        for (int i = 0; i < hands.getInventory().getContainerSize(); i++)
            if (hands.getInventory().getItem(i).is(item.asItem())) return null;
        return "items on the ground near " + near + ":" + (seen.length() == 0 ? " none" : seen.toString());
    }

    /** Place the first {@code item} from this player's inventory (keeping its data) like {@link #place}, then take it out of the inventory. */
    public BlockState placeFromInventory(ItemLike item, BlockPos pos, Direction standOn, Predicate<BlockState> expect) {
        var inv = hands.getInventory();
        for (int i = 0; i < inv.getContainerSize(); i++) {
            ItemStack s = inv.getItem(i);
            if (!s.is(item.asItem())) continue;
            BlockState placed = place(s.copyWithCount(1), pos, standOn, expect, null);
            s.shrink(1);
            return placed;
        }
        throw new IllegalStateException("no " + item.asItem() + " in the player's inventory");
    }

    // ------------------------------------------------------------ keyboard and mouse

    /**
     * Sit down at the computer at {@code terminal}: unless this player has its
     * screen open or stands within reach of it already, walk up to it and
     * right-click its screen (opening it, as a player does before typing). A
     * fake player's menu doesn't stay open, so what lets its keys in is the
     * host's range rule ({@code TerminalBlockEntity.canInteract}: within 8
     * blocks), exactly as for a player standing at the screen. What the game
     * said before is kept.
     */
    public void sitAt(BlockPos terminal) {
        if (hands.containerMenu instanceof TerminalMenu m && m.getBlockEntity() != null
                && m.getBlockEntity().getBlockPos().equals(terminal)) return;
        if (hands.level() == level && hands.position().distanceToSqr(Vec3.atCenterOf(terminal)) <= 5.0 * 5.0) return;
        BlockState s = level.getBlockState(terminal);
        if (!s.hasProperty(TerminalBlock.FACING)) throw new IllegalStateException("no computer screen at " + terminal + ": " + s);
        Direction face = s.getValue(TerminalBlock.FACING);
        Vec3 hit = faceCentre(terminal, face);
        lookFrom(hit.add(Vec3.atLowerCornerOf(face.getNormal()).scale(1.5)), hit);
        hands.setItemInHand(InteractionHand.MAIN_HAND, ItemStack.EMPTY);
        hands.gameMode.useItemOn(hands, level, ItemStack.EMPTY, InteractionHand.MAIN_HAND,
                new BlockHitResult(hit, face, terminal, false));
    }

    /** Close whatever screen is open (Escape). */
    public void closeScreen() {
        hands.closeContainer();
    }

    /**
     * Type {@code text} on display {@code display} of computer {@code computer},
     * as the client sends it: the console's key string ({@code TerminalInputPacket})
     * and, for the GUI, the printable text ({@code KeyInputPacket} TEXT, at most
     * 256 characters a packet). Both go through the packets' server handlers,
     * so the host's range and holder rules apply. Returns false if refused.
     */
    public boolean type(UUID computer, int display, String text) {
        boolean ok = TerminalInputPacket.deliver(hands, new TerminalInputPacket(computer, (byte) display, text));
        StringBuilder printable = new StringBuilder();
        text.codePoints().filter(c -> c >= 32 && c != 127).forEach(printable::appendCodePoint);
        for (int i = 0; i < printable.length(); i += 256) {
            ok &= KeyInputPacket.deliver(hands, KeyInputPacket.text(computer, (byte) display,
                    printable.substring(i, Math.min(printable.length(), i + 256))));
        }
        return ok;
    }

    /**
     * Press and release one key (a USB HID usage with the protocol's modifier
     * bits) the way the client sends it: the raw key down, the console's key
     * string for it ({@code console}, null for none), the raw key up.
     */
    public boolean press(UUID computer, int display, int hid, int mods, String console) {
        boolean ok = KeyInputPacket.deliver(hands, KeyInputPacket.key(computer, (byte) display, KeyInputPacket.DOWN, hid, mods));
        if (console != null) ok &= TerminalInputPacket.deliver(hands, new TerminalInputPacket(computer, (byte) display, console));
        KeyInputPacket.deliver(hands, KeyInputPacket.key(computer, (byte) display, KeyInputPacket.UP, hid, mods));
        return ok;
    }

    /** HID usage of Enter, Backspace and the letter keys. */
    public static final int HID_ENTER = 0x28, HID_BACKSPACE = 0x2A;

    public static int hidOf(char letter) {
        return 0x04 + (Character.toLowerCase(letter) - 'a');
    }

    /** Enter. */
    public boolean enter(UUID computer, int display) {
        return press(computer, display, HID_ENTER, 0, "\n");
    }

    /** Ctrl+{@code letter}: the raw chord for the GUI and the control character for the console. */
    public boolean ctrl(UUID computer, int display, char letter) {
        return press(computer, display, hidOf(letter), com.example.evanscomputermod.computer.ui.proto.UiProto.Mods.CTRL,
                String.valueOf((char) (Character.toLowerCase(letter) & 0x1f)));
    }

    /** A key chord the GUI reads (Alt+F4, Ctrl+Shift+V, Ctrl+Alt+Backspace): raw key events only. */
    public boolean chord(UUID computer, int display, int hid, int mods) {
        return press(computer, display, hid, mods, null);
    }

    /**
     * A left click at framebuffer pixel ({@code x}, {@code y}) of a display, as
     * the client sends it ({@code MouseInputPacket}: move, press, release).
     */
    public boolean click(UUID computer, int display, int x, int y) {
        boolean ok = true;
        for (byte[] e : new byte[][]{{MouseInputPacket.KIND_MOVE, 0}, {MouseInputPacket.KIND_DOWN, 1}, {MouseInputPacket.KIND_UP, 0}}) {
            ok &= MouseInputPacket.deliver(hands, new MouseInputPacket(computer, (byte) display, e[0], (short) x, (short) y,
                    e[1], (byte) 0, (byte) 0));
        }
        return ok;
    }

    /** A touch (press and release, no move) at a pixel: what a tap on the ePhone's screen sends. */
    public boolean tap(UUID computer, int display, int x, int y) {
        boolean ok = MouseInputPacket.deliver(hands, new MouseInputPacket(computer, (byte) display, MouseInputPacket.KIND_DOWN,
                (short) x, (short) y, (byte) 1, (byte) 0, (byte) 0));
        ok &= MouseInputPacket.deliver(hands, new MouseInputPacket(computer, (byte) display, MouseInputPacket.KIND_UP,
                (short) x, (short) y, (byte) 0, (byte) 0, (byte) 0));
        return ok;
    }

    /** Run a chat command as this player with cheats on (operator level), returning what it said. */
    public List<String> command(String command) {
        clearHeard();
        var src = hands.createCommandSourceStack().withPermission(4);
        level.getServer().getCommands().performPrefixedCommand(src, command.startsWith("/") ? command.substring(1) : command);
        return heard();
    }
}
//?}
