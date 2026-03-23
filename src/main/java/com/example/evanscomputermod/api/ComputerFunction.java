package com.example.evanscomputermod.api;

import java.lang.annotation.ElementType;
import java.lang.annotation.Retention;
import java.lang.annotation.RetentionPolicy;
import java.lang.annotation.Target;

/**
 * Marks a method within a {@link ComputerModule} class as callable from Python.
 * Methods may optionally accept a {@link ComputerContext} as their first parameter;
 * it will be injected automatically and is not visible to Python callers.
 */
@Target(ElementType.METHOD)
@Retention(RetentionPolicy.RUNTIME)
public @interface ComputerFunction {

    /** Override the Python function name. Default: camelCase method name converted to snake_case. */
    String value() default "";

    /** Description for visual block labels and help text. */
    String description() default "";

    /** If true, the method will be executed on the main server thread. */
    boolean mainThread() default false;
}
