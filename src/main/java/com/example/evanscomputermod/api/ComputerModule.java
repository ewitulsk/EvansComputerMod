package com.example.evanscomputermod.api;

import java.lang.annotation.ElementType;
import java.lang.annotation.Retention;
import java.lang.annotation.RetentionPolicy;
import java.lang.annotation.Target;

/**
 * Marks a class as a computer module whose methods can be exposed to Python.
 * The module name becomes the Python import name.
 *
 * <pre>
 * {@literal @}ComputerModule("golem")
 * public class GolemAPI {
 *     {@literal @}ComputerFunction(description = "Summon a golem")
 *     public boolean summon(ComputerContext ctx, String type) { ... }
 * }
 * </pre>
 *
 * Python users can then do:
 * <pre>
 * import golem
 * golem.summon("iron")
 * </pre>
 */
@Target(ElementType.TYPE)
@Retention(RetentionPolicy.RUNTIME)
public @interface ComputerModule {

    /** The Python module name (what users will {@code import}). */
    String value();

    /** Description for documentation and visual block category labels. */
    String description() default "";
}
