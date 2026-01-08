package com.example.customworld.stubgen.mock;

import java.lang.annotation.ElementType;
import java.lang.annotation.Retention;
import java.lang.annotation.RetentionPolicy;
import java.lang.annotation.Target;

/**
 * Mock annotation that mirrors CC:Tweaked's @LuaFunction annotation.
 * Used for testing the PeripheralScanner without requiring CC:Tweaked.
 */
@Retention(RetentionPolicy.RUNTIME)
@Target(ElementType.METHOD)
public @interface MockLuaFunction {
    /**
     * Alternative names for this function in Lua.
     * If empty, the method name is used.
     */
    String[] value() default {};
    
    /**
     * Whether this function should run on the main server thread.
     */
    boolean mainThread() default false;
}
