package com.example.evanscomputermod.computer;

import com.example.evanscomputermod.api.ITerminalOutput;

import java.util.ArrayList;
import java.util.List;

/**
 * Standalone terminal display state with no Minecraft dependencies.
 * Manages an 80x24 character buffer with cursor and scrollback.
 */
public class TerminalDisplay implements ITerminalOutput {

    public static final int DEFAULT_WIDTH = 80;
    public static final int DEFAULT_HEIGHT = 24;
    public static final int SCROLLBACK_SIZE = 1000;

    private final int width;
    private final int height;
    private final char[][] buffer;
    private final List<char[]> scrollbackBuffer = new ArrayList<>();
    private int cursorX = 0;
    private int cursorY = 0;

    public TerminalDisplay() {
        this(DEFAULT_WIDTH, DEFAULT_HEIGHT);
    }

    public TerminalDisplay(int width, int height) {
        this.width = width;
        this.height = height;
        this.buffer = new char[height][width];
        clearBuffer();
    }

    @Override
    public int getWidth() {
        return width;
    }

    @Override
    public int getHeight() {
        return height;
    }

    @Override
    public void write(String text) {
        for (char c : text.toCharArray()) {
            writeChar(c);
        }
    }

    @Override
    public void writeChar(char c) {
        if (c == '\n') {
            newLine();
        } else if (c == '\r') {
            cursorX = 0;
        } else if (c == '\b') {
            if (cursorX > 0) {
                cursorX--;
                buffer[cursorY][cursorX] = ' ';
            }
        } else {
            if (cursorX >= width) {
                newLine();
            }
            buffer[cursorY][cursorX] = c;
            cursorX++;
        }
    }

    @Override
    public void clearBuffer() {
        for (int y = 0; y < height; y++) {
            for (int x = 0; x < width; x++) {
                buffer[y][x] = ' ';
            }
        }
        cursorX = 0;
        cursorY = 0;
    }

    @Override
    public void setCursor(int x, int y) {
        this.cursorX = Math.max(0, Math.min(x, width - 1));
        this.cursorY = Math.max(0, Math.min(y, height - 1));
    }

    @Override
    public int getCursorX() {
        return cursorX;
    }

    @Override
    public int getCursorY() {
        return cursorY;
    }

    @Override
    public char getChar(int x, int y) {
        if (x >= 0 && x < width && y >= 0 && y < height) {
            return buffer[y][x];
        }
        return ' ';
    }

    @Override
    public String getLine(int y) {
        if (y >= 0 && y < height) {
            return new String(buffer[y]);
        }
        return "";
    }

    @Override
    public String getBufferAsString() {
        StringBuilder sb = new StringBuilder();
        for (int y = 0; y < height; y++) {
            sb.append(buffer[y]);
            if (y < height - 1) {
                sb.append('\n');
            }
        }
        return sb.toString();
    }

    @Override
    public void setBufferFromString(String content) {
        String[] lines = content.split("\n", -1);
        for (int y = 0; y < height; y++) {
            if (y < lines.length) {
                for (int x = 0; x < width; x++) {
                    buffer[y][x] = x < lines[y].length() ? lines[y].charAt(x) : ' ';
                }
            } else {
                for (int x = 0; x < width; x++) {
                    buffer[y][x] = ' ';
                }
            }
        }
    }

    @Override
    public int getScrollbackSize() {
        return scrollbackBuffer.size();
    }

    @Override
    public String getScrollbackLine(int index) {
        if (index >= 0 && index < scrollbackBuffer.size()) {
            return new String(scrollbackBuffer.get(index));
        }
        return "";
    }

    @Override
    public void clearScrollback() {
        scrollbackBuffer.clear();
    }

    private void newLine() {
        cursorX = 0;
        cursorY++;
        if (cursorY >= height) {
            scrollUp();
            cursorY = height - 1;
        }
    }

    private void scrollUp() {
        // Save the top line to scrollback before discarding
        char[] topLine = new char[width];
        System.arraycopy(buffer[0], 0, topLine, 0, width);
        scrollbackBuffer.add(topLine);

        // Limit scrollback buffer size
        while (scrollbackBuffer.size() > SCROLLBACK_SIZE) {
            scrollbackBuffer.remove(0);
        }

        // Move all lines up by one
        for (int y = 0; y < height - 1; y++) {
            System.arraycopy(buffer[y + 1], 0, buffer[y], 0, width);
        }
        // Clear the bottom line
        for (int x = 0; x < width; x++) {
            buffer[height - 1][x] = ' ';
        }
    }

    /**
     * Saves the scrollback buffer to a string for NBT serialization.
     */
    public String getScrollbackAsString() {
        if (scrollbackBuffer.isEmpty()) return "";
        StringBuilder sb = new StringBuilder();
        for (int i = 0; i < scrollbackBuffer.size(); i++) {
            sb.append(new String(scrollbackBuffer.get(i)));
            if (i < scrollbackBuffer.size() - 1) {
                sb.append("\n");
            }
        }
        return sb.toString();
    }

    /**
     * Restores the scrollback buffer from a string (NBT deserialization).
     */
    public void setScrollbackFromString(String data) {
        scrollbackBuffer.clear();
        if (data == null || data.isEmpty()) return;
        String[] lines = data.split("\n", -1);
        for (String line : lines) {
            if (scrollbackBuffer.size() >= SCROLLBACK_SIZE) break;
            char[] chars = new char[width];
            for (int x = 0; x < width; x++) {
                chars[x] = (x < line.length()) ? line.charAt(x) : ' ';
            }
            scrollbackBuffer.add(chars);
        }
    }
}
