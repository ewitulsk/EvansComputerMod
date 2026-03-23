package com.example.evanscomputermod.api;

/**
 * Capability for hosts that have a text display.
 * A headless computer (e.g., a redstone controller) would return null
 * for this from {@link IComputerHost#getTerminalOutput()}.
 */
public interface ITerminalOutput {

    int getWidth();

    int getHeight();

    void write(String text);

    void writeChar(char c);

    void clearBuffer();

    void setCursor(int x, int y);

    int getCursorX();

    int getCursorY();

    char getChar(int x, int y);

    String getLine(int y);

    String getBufferAsString();

    void setBufferFromString(String content);

    int getScrollbackSize();

    String getScrollbackLine(int index);

    void clearScrollback();
}
