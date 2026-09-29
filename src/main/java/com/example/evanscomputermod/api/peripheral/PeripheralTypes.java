package com.example.evanscomputermod.api.peripheral;

import org.jetbrains.annotations.Nullable;

import java.util.Collection;
import java.util.Collections;
import java.util.Map;
import java.util.concurrent.ConcurrentHashMap;

/**
 * Catalogue of known peripheral types, used by the visual editor to offer one
 * block per method and by {@code help}. Registering is optional: an
 * unregistered peripheral still works, it just has no visual blocks.
 * Register during common setup:
 * <pre>
 * PeripheralTypes.register("lamp", "Colored lamp", LampPeripheral.class);
 * </pre>
 */
public final class PeripheralTypes {

    public record Type(String name, String description, PeripheralMethods methods) {}

    private static final Map<String, Type> TYPES = new ConcurrentHashMap<>();

    private PeripheralTypes() {
    }

    public static void register(String type, String description, Class<? extends IPeripheral> implementation) {
        TYPES.put(type, new Type(type, description, PeripheralMethods.of(implementation)));
    }

    @Nullable
    public static Type get(String type) {
        return TYPES.get(type);
    }

    public static Collection<Type> all() {
        return Collections.unmodifiableCollection(TYPES.values());
    }
}
