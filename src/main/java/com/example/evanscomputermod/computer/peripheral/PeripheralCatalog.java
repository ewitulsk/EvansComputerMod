package com.example.evanscomputermod.computer.peripheral;

import com.example.evanscomputermod.api.peripheral.PeripheralMethods;
import com.example.evanscomputermod.api.peripheral.PeripheralTypes;

import java.util.ArrayList;
import java.util.Comparator;
import java.util.List;

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
 */
public final class PeripheralCatalog {

    /** The reserved {@code periph_methods} name that returns the catalog. */
    public static final String NAME = "@catalog";

    private PeripheralCatalog() {
    }

    /** The catalog as a value for {@link PeripheralValues#ok}. */
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
