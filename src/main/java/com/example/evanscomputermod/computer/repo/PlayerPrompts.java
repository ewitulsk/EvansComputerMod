package com.example.evanscomputermod.computer.repo;

import com.example.evanscomputermod.EcmConfig;
import com.mojang.brigadier.CommandDispatcher;
import com.mojang.brigadier.arguments.IntegerArgumentType;
import com.mojang.brigadier.arguments.StringArgumentType;
import net.minecraft.ChatFormatting;
import net.minecraft.commands.CommandSourceStack;
import net.minecraft.commands.Commands;
import net.minecraft.network.chat.ClickEvent;
import net.minecraft.network.chat.Component;
import net.minecraft.network.chat.MutableComponent;
import net.minecraft.server.level.ServerPlayer;

import java.util.Map;
import java.util.UUID;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicInteger;

/**
 * Things a computer asks its player outside the screen, in chat: a yes/no
 * question only that player's own click answers ({@code keystore_confirm}), and
 * a link that opens their real browser ({@code player_link}).
 *
 * <p>A program cannot answer its own question: the answer is the command
 * {@code /ecmconfirm <id> allow|deny}, run by the player who was asked (Minecraft
 * runs it when they click the button), and the id is not shown to the program.
 */
public final class PlayerPrompts {
    private record Pending(UUID player, CompletableFuture<Boolean> answer) {}

    private static final Map<Integer, Pending> PENDING = new ConcurrentHashMap<>();
    private static final AtomicInteger NEXT = new AtomicInteger(new java.security.SecureRandom().nextInt(1 << 20) + 1);

    public static final int MAX_TIMEOUT_MS = 120_000;

    private PlayerPrompts() {}

    private static ClickEvent runCommand(String command) {
        //? if >=26.1 {
        return new ClickEvent.RunCommand(command);
        //?} else
        /*return new ClickEvent(ClickEvent.Action.RUN_COMMAND, command);*/
    }

    private static ClickEvent openUrl(String url) {
        //? if >=26.1 {
        return new ClickEvent.OpenUrl(java.net.URI.create(url));
        //?} else
        /*return new ClickEvent(ClickEvent.Action.OPEN_URL, url);*/
    }

    private static MutableComponent button(String label, ChatFormatting color, ClickEvent click) {
        return Component.literal("[" + label + "]").withStyle(s -> s.withColor(color).withBold(true).withClickEvent(click));
    }

    /**
     * Ask {@code player} a question and wait for their click. Blocks the calling
     * (program) thread; the message is sent on the server thread.
     *
     * @return true for Allow; false for Deny or no answer within {@code timeoutMs}
     * @throws InterruptedException the program was stopped while waiting
     */
    public static boolean confirm(ServerPlayer player, String what, String question, int timeoutMs) throws InterruptedException {
        int id = NEXT.getAndAdd(1 + new java.security.SecureRandom().nextInt(97));
        CompletableFuture<Boolean> answer = new CompletableFuture<>();
        PENDING.put(id, new Pending(player.getUUID(), answer));
        var server = player.level().getServer();
        if (server == null) {
            PENDING.remove(id);
            return false;
        }
        String clean = question.replaceAll("[\\p{Cntrl}]", " ");
        server.execute(() -> player.sendSystemMessage(
                Component.literal(what + " asks: ").withStyle(ChatFormatting.GRAY)
                        .append(Component.literal(clean).withStyle(ChatFormatting.WHITE))
                        .append(" ")
                        .append(button("Allow", ChatFormatting.GREEN, runCommand("/ecmconfirm " + id + " allow")))
                        .append(" ")
                        .append(button("Deny", ChatFormatting.RED, runCommand("/ecmconfirm " + id + " deny")))));
        try {
            return answer.get(Math.max(1000, Math.min(timeoutMs, MAX_TIMEOUT_MS)), TimeUnit.MILLISECONDS);
        } catch (java.util.concurrent.TimeoutException | java.util.concurrent.ExecutionException e) {
            return false;
        } finally {
            PENDING.remove(id);
        }
    }

    /** {@code /ecmconfirm}: the asked player's answer. Returns a message for them. */
    static String answer(ServerPlayer by, int id, boolean allow) {
        Pending p = PENDING.get(id);
        if (p == null) return "That question is no longer open.";
        if (!p.player.equals(by.getUUID())) return "That question was not asked of you.";
        PENDING.remove(id);
        p.answer.complete(allow);
        return allow ? "Allowed." : "Denied.";
    }

    /** The open question asked of {@code player}, if any (scenario checks). */
    public static Integer pendingFor(UUID player) {
        return PENDING.entrySet().stream().filter(e -> e.getValue().player.equals(player)).map(Map.Entry::getKey).findFirst().orElse(null);
    }

    /**
     * Is {@code url} a page of the configured repository? Only those are shown
     * to players, so a program cannot send them anywhere else.
     */
    public static boolean linkAllowed(String url) {
        String origin = EcmConfig.repoUrl();
        if (origin.isEmpty() || url.length() > 512 || url.chars().anyMatch(c -> c <= 0x20 || c >= 0x7f)) return false;
        return url.equals(origin) || url.startsWith(origin + "/");
    }

    /** Put a clickable link to a repository page in {@code player}'s chat. */
    public static void link(ServerPlayer player, String url, String text) {
        var server = player.level().getServer();
        if (server == null) return;
        String label = text.replaceAll("[\\p{Cntrl}]", " ");
        String shown = label.isBlank() ? url : (label.length() > 80 ? label.substring(0, 80) : label);
        server.execute(() -> player.sendSystemMessage(
                Component.literal(shown + ": ").withStyle(ChatFormatting.GRAY)
                        .append(Component.literal(url).withStyle(s -> s.withColor(ChatFormatting.AQUA).withUnderlined(true).withClickEvent(openUrl(url))))));
    }

    /** {@code /ecmconfirm <id> allow|deny}: for every player (it only answers a question asked of them). */
    public static void register(CommandDispatcher<CommandSourceStack> dispatcher) {
        dispatcher.register(Commands.literal("ecmconfirm")
                .then(Commands.argument("id", IntegerArgumentType.integer(1))
                        .then(Commands.argument("answer", StringArgumentType.word())
                                .suggests((c, b) -> net.minecraft.commands.SharedSuggestionProvider.suggest(java.util.List.of("allow", "deny"), b))
                                .executes(c -> {
                                    ServerPlayer player = c.getSource().getPlayer();
                                    if (player == null) {
                                        c.getSource().sendFailure(Component.literal("Only the player who was asked can answer."));
                                        return 0;
                                    }
                                    String a = StringArgumentType.getString(c, "answer");
                                    if (!a.equals("allow") && !a.equals("deny")) {
                                        c.getSource().sendFailure(Component.literal("Answer allow or deny."));
                                        return 0;
                                    }
                                    String msg = answer(player, IntegerArgumentType.getInteger(c, "id"), a.equals("allow"));
                                    c.getSource().sendSuccess(() -> Component.literal(msg), false);
                                    return 1;
                                }))));
    }
}
