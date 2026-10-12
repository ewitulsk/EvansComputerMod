package com.example.evanscomputermod.computer.apps;

/**
 * Lifecycle and service messages the app manager needs delivered to apps over
 * their UI connections. The compositor implements this (it owns the
 * connections) and installs it with {@link AppManager#setAppEvents}; without
 * one (console mode) the defaults below run: nothing is delivered and apps
 * with no UI connection are simply not asked for state.
 *
 * <p>Answers come back through the app manager: {@code SaveState} →
 * {@link AppManager#onSaveState}, {@code BackgroundDone} →
 * {@link AppManager#onBackgroundDone}, {@code FilePicked} →
 * {@link AppManager#onFilePicked}, {@code Open} → {@link AppManager#open}.
 * When an app's connection says Hello, the compositor sets
 * {@code restore_follows} from {@link AppManager#peekRestore} and sends
 * {@link AppManager#takeRestore}'s bytes as {@code Restore} right after
 * {@code Welcome}.
 */
public interface AppEvents {

    /**
     * Send {@code WillSuspend{deadline_ms}} to every UI connection of
     * {@code app}. Return true if at least one connection got it (so a
     * {@code SaveState} may follow), false if the app has none.
     */
    default boolean willSuspend(AppInstance app, int deadlineMs) {
        return false;
    }

    /** Send {@code OpenFile{path}} (open with); the grant is already in place. */
    default void openFile(AppInstance app, String path) {}

    /**
     * A refresh wake starts for a suspended app (iPhone policy): it runs for
     * up to {@code budgetMs} or until it sends {@code BackgroundDone}.
     * Optional hint for the compositor; the app is resumed either way.
     */
    default void refreshWake(AppInstance app, long budgetMs) {}

    /** Background exception modes changed (status-bar indicators). Any thread. */
    default void backgroundModesChanged(AppInstance app, java.util.Set<String> activeModes) {}
}
