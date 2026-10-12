package com.example.evanscomputermod.client;

/**
 * A screen that hosts a computer's display (the Terminal GUI, the open ePhone).
 * {@link ComputerScreenInput} hands it the mouse over its display, and every key,
 * before any other mod's screen listeners see them: overlays such as JEI's act on
 * NeoForge's screen input events, which fire before the screen's own handlers, so
 * without this a click on a window's close button over JEI's (even hidden)
 * ingredient list opened a recipe instead.
 *
 * <p>Coordinates are GUI coordinates, buttons and keys GLFW codes; the methods
 * return whether the screen used the input (the event is consumed either way).
 */
public interface ComputerInputScreen {
    /** Whether the pointer at ({@code x}, {@code y}) is over this screen's own area (its display and controls). */
    boolean ownsPointer(double x, double y);

    boolean computerMousePressed(double x, double y, int button);

    boolean computerMouseReleased(double x, double y, int button);

    boolean computerMouseDragged(double x, double y, int button, double dx, double dy);

    boolean computerMouseScrolled(double x, double y, double scrollX, double scrollY);

    boolean computerKeyPressed(int key, int scan, int mods);

    boolean computerKeyReleased(int key, int scan, int mods);

    boolean computerCharTyped(int codePoint);

    /**
     * A key that must be swallowed without reaching the computer or anyone else
     * (the Terminal's Wireless Controller keys; ControllerClient drives the controller).
     */
    default boolean swallowsKey(int key) {
        return false;
    }

    default boolean swallowsChar(int codePoint) {
        return false;
    }
}
