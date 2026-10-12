package com.example.evanscomputermod.computer.apps;

import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.List;
import java.util.Map;
import java.util.TreeMap;

/**
 * The TOML subset {@code app.toml} is written in (docs/gui/APPS.md, "TOML
 * subset"); the same parser as {@code rust/wasm-programs/pkg/src/toml_lite.rs}.
 * Values are {@link String}, {@link Long}, {@link Boolean}, {@code List<Object>}
 * and {@code Map<String, Object>}.
 *
 * <p>Supported: {@code key = value} lines, {@code #} comments, one level of
 * {@code [table]} headers, basic and literal strings, decimal integers,
 * booleans, arrays (may span lines) and inline tables. Dotted keys, nested
 * headers, {@code [[arrays of tables]]}, multi-line strings, floats and dates
 * are syntax errors.
 */
public final class MiniToml {

    public static final class SyntaxException extends Exception {
        public SyntaxException(String msg) {
            super(msg);
        }
    }

    private final byte[] s;
    private int i;
    private int line = 1;

    private MiniToml(String text) {
        this.s = text.getBytes(StandardCharsets.UTF_8);
    }

    public static Map<String, Object> parse(String text) throws SyntaxException {
        return new MiniToml(text).document();
    }

    private Map<String, Object> document() throws SyntaxException {
        Map<String, Object> root = new TreeMap<>();
        Map<String, Object> target = root;
        while (true) {
            skipWsCommentsNewlines();
            if (i >= s.length) break;
            if (peek() == '[') {
                i++;
                if (peek() == '[') throw err("arrays of tables ([[...]]) are not supported");
                skipInlineWs();
                String name = key();
                skipInlineWs();
                if (peek() != ']') throw err("expected ']' after the table name");
                i++;
                endOfLine();
                if (root.containsKey(name)) throw err("duplicate key '" + name + "'");
                Map<String, Object> t = new TreeMap<>();
                root.put(name, t);
                target = t;
                continue;
            }
            String k = key();
            skipInlineWs();
            if (peek() != '=') throw err("expected '=' after '" + k + "'");
            i++;
            skipInlineWs();
            Object v = value();
            endOfLine();
            if (target.containsKey(k)) throw err("duplicate key '" + k + "'");
            target.put(k, v);
        }
        return root;
    }

    private int peek() {
        return i < s.length ? s[i] & 0xFF : -1;
    }

    private SyntaxException err(String msg) {
        return new SyntaxException("line " + line + ": " + msg);
    }

    private boolean startsWith(String lit) {
        byte[] b = lit.getBytes(StandardCharsets.UTF_8);
        if (i + b.length > s.length) return false;
        for (int k = 0; k < b.length; k++) if (s[i + k] != b[k]) return false;
        return true;
    }

    private void skipInlineWs() {
        while (peek() == ' ' || peek() == '\t') i++;
    }

    private void skipComment() {
        if (peek() == '#') {
            while (i < s.length && s[i] != '\n') i++;
        }
    }

    private void skipWsCommentsNewlines() {
        while (true) {
            skipInlineWs();
            skipComment();
            int c = peek();
            if (c == '\n') {
                line++;
                i++;
            } else if (c == '\r') {
                i++;
            } else {
                break;
            }
        }
    }

    private void endOfLine() throws SyntaxException {
        skipInlineWs();
        skipComment();
        int c = peek();
        if (c == -1 || c == '\n') return;
        if (c == '\r' && i + 1 < s.length && s[i + 1] == '\n') return;
        throw err("unexpected '" + (char) c + "' after the value");
    }

    private String key() throws SyntaxException {
        if (peek() == '"') return basicString();
        int start = i;
        while (true) {
            int c = peek();
            if ((c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z') || (c >= '0' && c <= '9') || c == '_' || c == '-') i++;
            else break;
        }
        if (i == start) throw err("expected a key");
        if (peek() == '.') throw err("dotted keys are not supported");
        return new String(s, start, i - start, StandardCharsets.UTF_8);
    }

    private Object value() throws SyntaxException {
        int c = peek();
        if (c == '"') {
            if (startsWith("\"\"\"")) throw err("multi-line strings are not supported");
            return basicString();
        }
        if (c == '\'') {
            if (startsWith("'''")) throw err("multi-line strings are not supported");
            i++;
            int start = i;
            while (i < s.length) {
                if (s[i] == '\'') {
                    String r = new String(s, start, i - start, StandardCharsets.UTF_8);
                    i++;
                    return r;
                }
                if (s[i] == '\n') break;
                i++;
            }
            throw err("unterminated string");
        }
        if (c == '[') {
            i++;
            List<Object> items = new ArrayList<>();
            while (true) {
                skipWsCommentsNewlines();
                if (peek() == ']') {
                    i++;
                    return items;
                }
                items.add(value());
                skipWsCommentsNewlines();
                if (peek() == ',') i++;
                else if (peek() != ']') throw err("expected ',' or ']' in an array");
            }
        }
        if (c == '{') {
            i++;
            Map<String, Object> t = new TreeMap<>();
            skipInlineWs();
            if (peek() == '}') {
                i++;
                return t;
            }
            while (true) {
                skipInlineWs();
                String k = key();
                skipInlineWs();
                if (peek() != '=') throw err("expected '=' after '" + k + "'");
                i++;
                skipInlineWs();
                Object v = value();
                if (t.containsKey(k)) throw err("duplicate key '" + k + "'");
                t.put(k, v);
                skipInlineWs();
                if (peek() == ',') {
                    i++;
                } else if (peek() == '}') {
                    i++;
                    return t;
                } else {
                    throw err("expected ',' or '}' in an inline table");
                }
            }
        }
        if (c == 't' && startsWith("true")) {
            i += 4;
            return Boolean.TRUE;
        }
        if (c == 'f' && startsWith("false")) {
            i += 5;
            return Boolean.FALSE;
        }
        if ((c >= '0' && c <= '9') || c == '-' || c == '+') {
            int start = i++;
            while ((peek() >= '0' && peek() <= '9') || peek() == '_') i++;
            int n = peek();
            if (n == '.' || n == 'e' || n == 'E' || n == ':') throw err("floats and dates are not supported");
            String txt = new String(s, start, i - start, StandardCharsets.UTF_8).replace("_", "");
            try {
                return Long.parseLong(txt.startsWith("+") ? txt.substring(1) : txt);
            } catch (NumberFormatException e) {
                throw err("bad integer '" + txt + "'");
            }
        }
        throw err("expected a value");
    }

    private String basicString() throws SyntaxException {
        i++; // opening quote
        java.io.ByteArrayOutputStream out = new java.io.ByteArrayOutputStream();
        while (i < s.length) {
            int c = s[i++] & 0xFF;
            if (c == '"') return out.toString(StandardCharsets.UTF_8);
            if (c == '\n') break;
            if (c != '\\') {
                out.write(c);
                continue;
            }
            if (i >= s.length) throw err("unterminated string");
            int e = s[i++] & 0xFF;
            switch (e) {
                case '"' -> out.write('"');
                case '\\' -> out.write('\\');
                case 'n' -> out.write('\n');
                case 't' -> out.write('\t');
                case 'r' -> out.write('\r');
                case 'u' -> {
                    if (i + 4 > s.length) throw err("bad \\u escape");
                    int cp;
                    try {
                        cp = Integer.parseInt(new String(s, i, 4, StandardCharsets.US_ASCII), 16);
                    } catch (NumberFormatException ex) {
                        throw err("bad \\u escape");
                    }
                    if (cp >= 0xD800 && cp <= 0xDFFF) throw err("bad \\u escape");
                    out.writeBytes(new String(Character.toChars(cp)).getBytes(StandardCharsets.UTF_8));
                    i += 4;
                }
                default -> throw err("unknown escape '\\" + (char) e + "'");
            }
        }
        throw err("unterminated string");
    }
}
