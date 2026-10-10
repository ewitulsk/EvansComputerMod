package com.example.evanscomputermod.testing.scenario;

//? if <=1.21.1 {
import com.example.evanscomputermod.block.ModBlocks;
import com.example.evanscomputermod.block.TerminalBlock;
import com.example.evanscomputermod.item.ModItems;
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
 * keyboard, or a command the player could type. Screens (the Access Point
 * GUI) are driven by calling the same server-side handler their packets call.
 */
public final class ScenarioPlayer {
    public static final String NAME = "ScenarioBuilder";
    private static final UUID ID = UUID.nameUUIDFromBytes(("ecm-scenario:" + NAME).getBytes());

    private final ServerLevel level;
    private final Hands hands;

    /** A fake player that keeps what the game says to it (messages may arrive from solver threads). */
    private static final class Hands extends FakePlayer {
        final List<String> heard = new CopyOnWriteArrayList<>();

        Hands(ServerLevel level) {
            super(level, new GameProfile(ID, NAME));
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
        this.level = level;
        this.hands = new Hands(level);
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
        hands.closeContainer();
        String reply = ApPackets.configure(hands, new ApPackets.Configure(ap, ApPackets.Action.APPLY, settings,
                passphrase == null ? "" : passphrase, 0));
        hands.heard.add(reply);
        return reply;
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

    /** Run a chat command as this player with cheats on (operator level), returning what it said. */
    public List<String> command(String command) {
        clearHeard();
        var src = hands.createCommandSourceStack().withPermission(4);
        level.getServer().getCommands().performPrefixedCommand(src, command.startsWith("/") ? command.substring(1) : command);
        return heard();
    }
}
//?}
