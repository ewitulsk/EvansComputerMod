package com.example.evanscomputermod.computer.peripheral;

import com.example.evanscomputermod.api.peripheral.PeripheralMethods;
import com.example.evanscomputermod.api.peripheral.PeripheralTypes;

import com.example.evanscomputermod.api.peripheral.IPeripheral;

import java.util.ArrayList;
import java.util.Collection;
import java.util.Comparator;
import java.util.List;
import java.util.Map;
import java.util.TreeMap;
import java.util.concurrent.ConcurrentHashMap;

/**
 * Every registered peripheral type with its methods, as programs see them:
 * the Visual Programmer turns this into one block per method (the type's
 * category), and its Package step infers {@code peripherals:<type>}
 * permissions from it. Programs read it with
 * {@code periph_methods("@catalog")} ({@code ecm_host_abi::peripheral::catalog}).
 *
 * <p>Encoded value (sorted by type, methods by name):
 * {@code [[type, description, [[method, description, [[param, port_type, default], ...], return_port_type], ...]], ...]}
 * where port types are the Visual Programmer's {@code string}, {@code number}
 * and {@code boolean}, and the return type is {@code ""} for void methods.
 *
 * <p>Besides the registered types, the catalog has the types of the
 * peripherals attached to the computer asking (a block with an inventory and
 * a tank lists its tank methods under {@code inventory} too; a peripheral of a
 * type nobody registered, such as one from another mod, is listed by the
 * methods it describes, see {@link IPeripheral#describeMethods}), and every
 * type the server has seen attached since it started, so a program's blocks
 * stay in the palette when its peripheral is moved away. A method nothing
 * describes is {@code [[args, any, ""]]} returning {@code any}.
 */
public final class PeripheralCatalog {

    /** The reserved {@code periph_methods} name that returns the catalog. */
    public static final String NAME = "@catalog";

    private PeripheralCatalog() {
    }

    /** Types seen attached anywhere since the server started: type -> method name -> catalog entry. */
    private static final Map<String, Map<String, List<Object>>> SEEN = new ConcurrentHashMap<>();

    /** The catalog as a value for {@link PeripheralValues#ok}, with what is attached as well. */
    public static List<Object> describe(Collection<IPeripheral> attached) {
        for (IPeripheral p : attached) {
            Map<String, List<Object>> seen = SEEN.computeIfAbsent(p.getType(), t -> new ConcurrentHashMap<>());
            List<PeripheralMethods.Info> infos = p.describeMethods();
            if (infos != null) {
                for (PeripheralMethods.Info m : infos) seen.putIfAbsent(m.name(), entry(m));
            } else {
                for (String m : p.getMethodNames()) seen.putIfAbsent(m, List.of(m, "", List.of(List.of("args", "any", "")), "any"));
            }
        }
        // type -> [description, method -> entry], sorted.
        Map<String, String> descriptions = new TreeMap<>();
        Map<String, Map<String, List<Object>>> methods = new TreeMap<>();
        for (PeripheralTypes.Type type : PeripheralTypes.all()) {
            descriptions.put(type.name(), type.description());
            Map<String, List<Object>> ms = methods.computeIfAbsent(type.name(), t -> new TreeMap<>());
            for (PeripheralMethods.Info m : type.methods().infos()) ms.put(m.name(), entry(m));
        }
        SEEN.forEach((type, seen) -> {
            descriptions.putIfAbsent(type, "");
            Map<String, List<Object>> ms = methods.computeIfAbsent(type, t -> new TreeMap<>());
            seen.forEach(ms::putIfAbsent);
        });
        List<Object> out = new ArrayList<>();
        methods.forEach((type, ms) -> out.add(List.of(type, descriptions.get(type), new ArrayList<>(ms.values()))));
        return out;
    }

    private static List<Object> entry(PeripheralMethods.Info m) {
        List<Object> params = new ArrayList<>();
        for (PeripheralMethods.Param p : m.params()) params.add(List.of(p.name(), portType(p.type()), defaultFor(p.type())));
        boolean returns = m.returnType() != void.class && m.returnType() != Void.class;
        return List.of(m.name(), m.description(), params, returns ? portType(m.returnType()) : "");
    }

    /** The catalog of the registered types only. */
    public static List<Object> describe() {
        List<PeripheralTypes.Type> types = new ArrayList<>(PeripheralTypes.all());
        types.sort(Comparator.comparing(PeripheralTypes.Type::name));
        List<Object> out = new ArrayList<>();
        for (PeripheralTypes.Type type : types) {
            List<PeripheralMethods.Info> infos = new ArrayList<>(type.methods().infos());
            infos.sort(Comparator.comparing(PeripheralMethods.Info::name));
            List<Object> methods = new ArrayList<>();
            for (PeripheralMethods.Info m : infos) {
                List<Object> params = new ArrayList<>();
                for (PeripheralMethods.Param p : m.params()) {
                    params.add(List.of(p.name(), portType(p.type()), defaultFor(p.type())));
                }
                boolean returns = m.returnType() != void.class && m.returnType() != Void.class;
                methods.add(List.of(m.name(), m.description(), params, returns ? portType(m.returnType()) : ""));
            }
            out.add(List.of(type.name(), type.description(), methods));
        }
        return out;
    }

    /** Encoded result frame of {@link #describe()}. */
    public static byte[] encoded() {
        return PeripheralValues.ok(describe());
    }

    /** Encoded result frame of {@link #describe(Collection)}. */
    public static byte[] encoded(Collection<IPeripheral> attached) {
        return PeripheralValues.ok(describe(attached));
    }

    /** The Visual Programmer port type of a Java parameter / return type. */
    public static String portType(Class<?> type) {
        if (type == int.class || type == Integer.class || type == long.class || type == Long.class
                || type == float.class || type == Float.class || type == double.class || type == Double.class) {
            return "number";
        }
        if (type == boolean.class || type == Boolean.class) return "boolean";
        return "string";
    }

    /** The default value shown for an unconnected port of this type. */
    public static String defaultFor(Class<?> type) {
        if (type == int.class || type == Integer.class || type == long.class || type == Long.class) return "0";
        if (type == float.class || type == Float.class || type == double.class || type == Double.class) return "0.0";
        if (type == boolean.class || type == Boolean.class) return "False";
        return "";
    }
}
