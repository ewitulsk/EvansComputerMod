package com.example.evanscomputermod.api.peripheral;

import org.jetbrains.annotations.Nullable;

import java.lang.reflect.InvocationTargetException;
import java.lang.reflect.Method;
import java.lang.reflect.Modifier;
import java.lang.reflect.Parameter;
import java.util.ArrayList;
import java.util.Collection;
import java.util.Collections;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Set;

/**
 * The {@link PeripheralMethod} methods of a class, found by reflection once
 * per class, plus argument conversion and dispatch. {@link AnnotatedPeripheral}
 * uses this; a peripheral with its own base class can hold one directly:
 * <pre>
 * private static final PeripheralMethods METHODS = PeripheralMethods.of(MyPeripheral.class);
 * public Set&lt;String&gt; getMethodNames() { return METHODS.names(); }
 * public Object callMethod(IComputerAccess c, String m, Object[] a) throws PeripheralException {
 *     return METHODS.call(this, c, m, a);
 * }
 * </pre>
 */
public final class PeripheralMethods {

    /** A parameter as programs see it (the injected {@link IComputerAccess} is not listed). */
    public record Param(String name, Class<?> type, boolean optional) {}

    /** Metadata for one callable method, used by dispatch, {@code help} and the visual editor. */
    public record Info(String name, String description, List<Param> params, Class<?> returnType, boolean mainThread) {}

    private record Entry(Info info, Method method, boolean wantsComputer) {}

    private static final ClassValue<PeripheralMethods> CACHE = new ClassValue<>() {
        @Override
        protected PeripheralMethods computeValue(Class<?> type) {
            return new PeripheralMethods(type);
        }
    };

    private final Map<String, Entry> entries;
    private final Set<String> names;

    public static PeripheralMethods of(Class<?> type) {
        return CACHE.get(type);
    }

    private PeripheralMethods(Class<?> type) {
        Map<String, Entry> found = new LinkedHashMap<>();
        for (Method method : type.getMethods()) {
            PeripheralMethod annotation = method.getAnnotation(PeripheralMethod.class);
            if (annotation == null || Modifier.isStatic(method.getModifiers())) continue;

            String name = annotation.value().isEmpty() ? camelToSnake(method.getName()) : annotation.value();
            Parameter[] parameters = method.getParameters();
            boolean wantsComputer = parameters.length > 0
                    && IComputerAccess.class.isAssignableFrom(parameters[0].getType());
            List<Param> params = new ArrayList<>();
            for (int i = wantsComputer ? 1 : 0; i < parameters.length; i++) {
                Class<?> t = parameters[i].getType();
                params.add(new Param(camelToSnake(parameters[i].getName()), t, !t.isPrimitive()));
            }
            Info info = new Info(name, annotation.description(), List.copyOf(params),
                    method.getReturnType(), annotation.mainThread());
            if (found.put(name, new Entry(info, method, wantsComputer)) != null) {
                throw new IllegalStateException(type.getName() + " declares peripheral method '" + name + "' twice");
            }
        }
        this.entries = Collections.unmodifiableMap(found);
        this.names = Collections.unmodifiableSet(found.keySet());
    }

    public Set<String> names() {
        return names;
    }

    public Collection<Info> infos() {
        return Collections.unmodifiableCollection(entries.values().stream().map(Entry::info).toList());
    }

    @Nullable
    public Info info(String method) {
        Entry e = entries.get(method);
        return e == null ? null : e.info();
    }

    public boolean isMainThread(String method) {
        Entry e = entries.get(method);
        return e == null || e.info().mainThread();
    }

    /** Convert {@code args} to {@code method}'s parameter types and invoke it on {@code target}. */
    @Nullable
    public Object call(Object target, IComputerAccess computer, String method, Object[] args) throws PeripheralException {
        Entry entry = entries.get(method);
        if (entry == null) {
            throw new PeripheralException("no such method '" + method + "'");
        }
        List<Param> params = entry.info().params();
        if (args.length > params.size()) {
            throw new PeripheralException(method + " takes at most " + params.size()
                    + " argument" + (params.size() == 1 ? "" : "s") + ", got " + args.length);
        }
        int offset = entry.wantsComputer() ? 1 : 0;
        Object[] javaArgs = new Object[params.size() + offset];
        if (entry.wantsComputer()) javaArgs[0] = computer;
        for (int i = 0; i < params.size(); i++) {
            Object value = i < args.length ? args[i] : null;
            javaArgs[i + offset] = convert(method, i, params.get(i), value);
        }
        try {
            return entry.method().invoke(target, javaArgs);
        } catch (InvocationTargetException e) {
            Throwable cause = e.getCause();
            if (cause instanceof PeripheralException pe) throw pe;
            if (cause instanceof RuntimeException re) throw re;
            throw new RuntimeException(cause);
        } catch (IllegalAccessException e) {
            throw new IllegalStateException("peripheral method " + method + " is not accessible", e);
        }
    }

    @Nullable
    private static Object convert(String method, int index, Param param, @Nullable Object value) throws PeripheralException {
        Class<?> t = param.type();
        if (value == null) {
            if (t.isPrimitive()) {
                throw badArgument(method, index, param, "no value");
            }
            return null;
        }
        if (t == Object.class) return value;
        if (t == int.class || t == Integer.class) return (int) integral(method, index, param, value, Integer.MIN_VALUE, Integer.MAX_VALUE);
        if (t == long.class || t == Long.class) return integral(method, index, param, value, Long.MIN_VALUE, Long.MAX_VALUE);
        if (t == double.class || t == Double.class) return number(method, index, param, value);
        if (t == float.class || t == Float.class) return (float) number(method, index, param, value);
        if (t == boolean.class || t == Boolean.class) {
            if (value instanceof Boolean b) return b;
            throw badArgument(method, index, param, typeName(value));
        }
        if (t == String.class) {
            if (value instanceof String s) return s;
            throw badArgument(method, index, param, typeName(value));
        }
        if (t == byte[].class) {
            if (value instanceof byte[] b) return b;
            if (value instanceof String s) return s.getBytes(java.nio.charset.StandardCharsets.UTF_8);
            throw badArgument(method, index, param, typeName(value));
        }
        if (t.isInstance(value)) return value;
        throw badArgument(method, index, param, typeName(value));
    }

    private static long integral(String method, int index, Param param, Object value, long min, long max) throws PeripheralException {
        long v;
        if (value instanceof Long || value instanceof Integer || value instanceof Short || value instanceof Byte) {
            v = ((Number) value).longValue();
        } else if (value instanceof Double || value instanceof Float) {
            double d = ((Number) value).doubleValue();
            if (d != Math.rint(d) || Double.isInfinite(d)) {
                throw new PeripheralException("bad argument #" + (index + 1) + " (" + param.name()
                        + ") to " + method + ": expected an integer, got " + d);
            }
            v = (long) d;
        } else {
            throw badArgument(method, index, param, typeName(value));
        }
        if (v < min || v > max) {
            throw new PeripheralException("bad argument #" + (index + 1) + " (" + param.name()
                    + ") to " + method + ": " + v + " is out of range");
        }
        return v;
    }

    private static double number(String method, int index, Param param, Object value) throws PeripheralException {
        if (value instanceof Number n && !(value instanceof Boolean)) return n.doubleValue();
        throw badArgument(method, index, param, typeName(value));
    }

    private static PeripheralException badArgument(String method, int index, Param param, String got) {
        return new PeripheralException("bad argument #" + (index + 1) + " (" + param.name() + ") to "
                + method + ": expected " + typeName(param.type()) + ", got " + got);
    }

    /** Program-facing name of a Java parameter type. */
    public static String typeName(Class<?> t) {
        if (t == int.class || t == Integer.class || t == long.class || t == Long.class) return "int";
        if (t == double.class || t == Double.class || t == float.class || t == Float.class) return "float";
        if (t == boolean.class || t == Boolean.class) return "bool";
        if (t == String.class) return "str";
        if (t == byte[].class) return "bytes";
        if (List.class.isAssignableFrom(t) || Collection.class.isAssignableFrom(t)) return "list";
        if (Map.class.isAssignableFrom(t)) return "dict";
        if (t == void.class || t == Void.class) return "None";
        return "any";
    }

    private static String typeName(Object value) {
        if (value instanceof Boolean) return "bool";
        if (value instanceof Long || value instanceof Integer) return "int";
        if (value instanceof Double) return "float";
        if (value instanceof String) return "str";
        if (value instanceof byte[]) return "bytes";
        if (value instanceof List) return "list";
        if (value instanceof Map) return "dict";
        return value.getClass().getSimpleName();
    }

    static String camelToSnake(String name) {
        StringBuilder sb = new StringBuilder(name.length() + 4);
        for (int i = 0; i < name.length(); i++) {
            char c = name.charAt(i);
            if (Character.isUpperCase(c)) {
                if (i > 0) sb.append('_');
                sb.append(Character.toLowerCase(c));
            } else {
                sb.append(c);
            }
        }
        return sb.toString();
    }
}
