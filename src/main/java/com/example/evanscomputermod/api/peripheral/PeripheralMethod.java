package com.example.evanscomputermod.api.peripheral;

import java.lang.annotation.ElementType;
import java.lang.annotation.Retention;
import java.lang.annotation.RetentionPolicy;
import java.lang.annotation.Target;

/**
 * Marks a public method of an {@link AnnotatedPeripheral} (or any class used
 * with {@link PeripheralMethods}) as callable from programs.
 *
 * <p>The method may take an {@link IComputerAccess} as its first parameter; it
 * is injected and not visible to callers. Other parameters are converted from
 * the program's values: {@code int}, {@code long}, {@code double},
 * {@code float}, {@code boolean} (and their boxes), {@link String},
 * {@code byte[]}, {@link java.util.List}, {@link java.util.Map} and
 * {@link Object}. Boxed and reference parameters are optional: a missing
 * trailing argument arrives as {@code null}.
 */
@Target(ElementType.METHOD)
@Retention(RetentionPolicy.RUNTIME)
public @interface PeripheralMethod {

    /** Name programs call. Default: the Java method name converted to snake_case. */
    String value() default "";

    /** One-line description, used by {@code help} and the visual editor. */
    String description() default "";

    /**
     * Run on the server thread (the default). Set false only for methods that
     * are thread-safe and don't touch the world; they then run directly on
     * the program's thread without waiting for the next server tick.
     */
    boolean mainThread() default true;
}
