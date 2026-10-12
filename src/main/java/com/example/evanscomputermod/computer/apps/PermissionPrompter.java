package com.example.evanscomputermod.computer.apps;

import org.jetbrains.annotations.Nullable;

import java.util.concurrent.CompletableFuture;

/**
 * Shows "Allow &lt;app&gt; to use &lt;permission&gt;?" to the player. The
 * compositor implements it (by sending {@code PermissionPrompt} to the shell
 * and completing the future from {@code AnswerPermission}) and installs it
 * with {@link AppManager#setPrompter}. With no prompter (console mode) the app
 * manager falls back to its no-prompter policy (deny by default; see
 * {@code EcmConfig} {@code apps.consolePermissions}).
 *
 * <p>Called on the app's own thread, which waits for the answer (up to the
 * prompt timeout, then the request is denied without remembering). Don't
 * block in {@link #prompt}; return a future and complete it later.
 */
@FunctionalInterface
public interface PermissionPrompter {

    /** One question. {@code req} is unique per computer (the wire {@code req} of PermissionPrompt). */
    record Request(int req, String appId, String appName, String permission, int pid) {}

    /** The player's answer: {@code remember = false} applies to this app run only. */
    record Answer(boolean allow, boolean remember) {
        public static final Answer DENY_ONCE = new Answer(false, false);
    }

    /**
     * Ask, or return null when no prompt can be shown right now (no shell
     * running): the request then gets the no-prompter policy.
     */
    @Nullable
    CompletableFuture<Answer> prompt(Request request);
}
