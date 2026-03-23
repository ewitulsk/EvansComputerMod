package com.example.evanscomputermod.wasm;

import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.api.*;

import java.io.ByteArrayOutputStream;
import java.lang.reflect.InvocationTargetException;
import java.lang.reflect.Method;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.nio.charset.StandardCharsets;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.ExecutionException;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.TimeoutException;

import net.minecraft.server.MinecraftServer;

/**
 * Invokes methods registered via {@link ComputerModule} annotations.
 * Uses a binary protocol for argument passing and result serialization,
 * eliminating JSON overhead.
 *
 * <h3>Binary Wire Format</h3>
 * <pre>
 * Arguments: [u8 arg_count] ([u8 type_tag] [payload])*
 * Result:    [u8 status(0=ok,1=err)] [u8 type_tag] [payload]
 *
 * Type tags:
 *   0x00 = null
 *   0x01 = string:  [u32 len] [utf8 bytes]
 *   0x02 = i32:     [i32 LE]
 *   0x03 = i64:     [i64 LE]
 *   0x04 = f64:     [f64 LE]
 *   0x05 = bool:    [u8 0|1]
 * </pre>
 */
public class ModuleMethodInvoker {

    // Type tags — must match Rust side
    public static final byte TAG_NULL   = 0x00;
    public static final byte TAG_STRING = 0x01;
    public static final byte TAG_I32    = 0x02;
    public static final byte TAG_I64    = 0x03;
    public static final byte TAG_F64    = 0x04;
    public static final byte TAG_BOOL   = 0x05;

    // Result status
    public static final byte STATUS_OK    = 0x00;
    public static final byte STATUS_ERROR = 0x01;

    /**
     * Invokes a registered module method using binary-encoded arguments.
     *
     * @param host       The computer host (for ComputerContext)
     * @param moduleName The Python module name
     * @param methodName The Python function name
     * @param argsBinary Binary-encoded arguments
     * @return Binary-encoded result
     */
    public byte[] invokeMethod(IComputerHost host, String moduleName, String methodName, byte[] argsBinary) {
        ComputerModuleRegistry.ModuleRegistration module = ComputerModuleRegistry.getModule(moduleName);
        if (module == null) {
            return serializeError("Module not found: " + moduleName);
        }

        ComputerModuleRegistry.MethodRegistration methodReg = module.methods.get(methodName);
        if (methodReg == null) {
            return serializeError("Method not found: " + moduleName + "." + methodName);
        }

        try {
            // Parse binary arguments
            Object[] rawArgs = parseBinaryArgs(argsBinary);

            // Build actual argument array
            Method javaMethod = methodReg.javaMethod;
            Object[] callArgs;

            if (methodReg.needsContext) {
                callArgs = new Object[methodReg.params.length + 1];
                callArgs[0] = new ComputerContext(host);
                for (int i = 0; i < methodReg.params.length; i++) {
                    callArgs[i + 1] = convertArg(i < rawArgs.length ? rawArgs[i] : null, methodReg.params[i].type);
                }
            } else {
                callArgs = new Object[methodReg.params.length];
                for (int i = 0; i < methodReg.params.length; i++) {
                    callArgs[i] = convertArg(i < rawArgs.length ? rawArgs[i] : null, methodReg.params[i].type);
                }
            }

            // Execute (optionally on main thread)
            Object result;
            if (methodReg.mainThread) {
                result = executeOnMainThread(host, module.instance, javaMethod, callArgs);
            } else {
                result = javaMethod.invoke(module.instance, callArgs);
            }

            return serializeResult(result);

        } catch (InvocationTargetException e) {
            Throwable cause = e.getCause();
            EvansComputerMod.LOGGER.error("Error invoking {}.{}", moduleName, methodName, cause);
            return serializeError(cause != null ? cause.getMessage() : e.getMessage());
        } catch (Exception e) {
            EvansComputerMod.LOGGER.error("Error invoking {}.{}", moduleName, methodName, e);
            return serializeError(e.getMessage());
        }
    }

    private Object executeOnMainThread(IComputerHost host, Object instance, Method method, Object[] args)
            throws Exception {
        MinecraftServer server = host.getServer();
        if (server == null) {
            return method.invoke(instance, args);
        }

        CompletableFuture<Object> future = new CompletableFuture<>();
        server.execute(() -> {
            try {
                future.complete(method.invoke(instance, args));
            } catch (Exception e) {
                future.completeExceptionally(e);
            }
        });

        try {
            return future.get(10, TimeUnit.SECONDS);
        } catch (ExecutionException e) {
            throw (Exception) e.getCause();
        } catch (TimeoutException e) {
            throw new RuntimeException("Main thread execution timed out");
        }
    }

    // ==================== Binary Argument Parsing ====================

    /**
     * Parses binary-encoded arguments into an Object array.
     * Format: [u8 arg_count] ([u8 type_tag] [payload])*
     */
    static Object[] parseBinaryArgs(byte[] data) {
        if (data == null || data.length == 0) return new Object[0];

        ByteBuffer buf = ByteBuffer.wrap(data).order(ByteOrder.LITTLE_ENDIAN);
        int argCount = buf.get() & 0xFF;
        Object[] args = new Object[argCount];

        for (int i = 0; i < argCount && buf.hasRemaining(); i++) {
            byte tag = buf.get();
            args[i] = readTaggedValue(buf, tag);
        }

        return args;
    }

    private static Object readTaggedValue(ByteBuffer buf, byte tag) {
        return switch (tag) {
            case TAG_NULL -> null;
            case TAG_STRING -> {
                int len = buf.getInt();
                byte[] bytes = new byte[len];
                buf.get(bytes);
                yield new String(bytes, StandardCharsets.UTF_8);
            }
            case TAG_I32 -> buf.getInt();
            case TAG_I64 -> buf.getLong();
            case TAG_F64 -> buf.getDouble();
            case TAG_BOOL -> (buf.get() != 0);
            default -> null;
        };
    }

    // ==================== Binary Result Serialization ====================

    /**
     * Serializes a successful result to binary.
     * Format: [0x00 status] [type_tag] [payload]
     */
    public static byte[] serializeResult(Object result) {
        ByteArrayOutputStream out = new ByteArrayOutputStream();
        out.write(STATUS_OK);
        writeTaggedValue(out, result);
        return out.toByteArray();
    }

    /**
     * Serializes an error to binary.
     * Format: [0x01 status] [0x01 string tag] [u32 len] [utf8 message]
     */
    public static byte[] serializeError(String message) {
        ByteArrayOutputStream out = new ByteArrayOutputStream();
        out.write(STATUS_ERROR);
        writeString(out, message != null ? message : "Unknown error");
        return out.toByteArray();
    }

    private static void writeTaggedValue(ByteArrayOutputStream out, Object value) {
        if (value == null) {
            out.write(TAG_NULL);
        } else if (value instanceof Boolean b) {
            out.write(TAG_BOOL);
            out.write(b ? 1 : 0);
        } else if (value instanceof Integer i) {
            out.write(TAG_I32);
            writeI32(out, i);
        } else if (value instanceof Long l) {
            out.write(TAG_I64);
            writeI64(out, l);
        } else if (value instanceof Float f) {
            out.write(TAG_F64);
            writeF64(out, f.doubleValue());
        } else if (value instanceof Double d) {
            out.write(TAG_F64);
            writeF64(out, d);
        } else if (value instanceof String s) {
            out.write(TAG_STRING);
            writeString(out, s);
        } else {
            // Fallback: convert to string
            out.write(TAG_STRING);
            writeString(out, value.toString());
        }
    }

    private static void writeString(ByteArrayOutputStream out, String s) {
        byte[] bytes = s.getBytes(StandardCharsets.UTF_8);
        writeI32(out, bytes.length);
        out.write(bytes, 0, bytes.length);
    }

    private static void writeI32(ByteArrayOutputStream out, int value) {
        out.write(value & 0xFF);
        out.write((value >> 8) & 0xFF);
        out.write((value >> 16) & 0xFF);
        out.write((value >> 24) & 0xFF);
    }

    private static void writeI64(ByteArrayOutputStream out, long value) {
        for (int i = 0; i < 8; i++) {
            out.write((int) ((value >> (i * 8)) & 0xFF));
        }
    }

    private static void writeF64(ByteArrayOutputStream out, double value) {
        writeI64(out, Double.doubleToRawLongBits(value));
    }

    // ==================== Type Conversion ====================

    static Object convertArg(Object value, Class<?> targetType) {
        if (value == null) {
            if (targetType.isPrimitive()) {
                return getDefaultPrimitive(targetType);
            }
            return null;
        }

        if (targetType == String.class) {
            return value.toString();
        }
        if (targetType == int.class || targetType == Integer.class) {
            if (value instanceof Number) return ((Number) value).intValue();
            return Integer.parseInt(value.toString());
        }
        if (targetType == long.class || targetType == Long.class) {
            if (value instanceof Number) return ((Number) value).longValue();
            return Long.parseLong(value.toString());
        }
        if (targetType == float.class || targetType == Float.class) {
            if (value instanceof Number) return ((Number) value).floatValue();
            return Float.parseFloat(value.toString());
        }
        if (targetType == double.class || targetType == Double.class) {
            if (value instanceof Number) return ((Number) value).doubleValue();
            return Double.parseDouble(value.toString());
        }
        if (targetType == boolean.class || targetType == Boolean.class) {
            if (value instanceof Boolean) return value;
            return Boolean.parseBoolean(value.toString());
        }

        return value;
    }

    private static Object getDefaultPrimitive(Class<?> type) {
        if (type == int.class) return 0;
        if (type == long.class) return 0L;
        if (type == float.class) return 0.0f;
        if (type == double.class) return 0.0;
        if (type == boolean.class) return false;
        return 0;
    }
}
