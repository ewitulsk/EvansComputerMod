package com.example.evanscomputermod.computer.browser;

import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

import java.lang.invoke.MethodHandle;
import java.lang.invoke.MethodHandles;
import java.lang.invoke.MethodType;
import java.lang.reflect.Field;
import java.lang.reflect.Method;

/**
 * Opens {@code java.desktop/sun.awt.*} (and friends) to JOGL at runtime so
 * JOGL's {@code Win32SunJDKReflection} / {@code X11SunJDKReflection} static
 * initializers — which call {@code setAccessible(true)} on
 * {@code sun.awt.Win32GraphicsConfig.getConfig} /
 * {@code sun.awt.X11GraphicsDevice.getScreen} — don't throw
 * {@code InaccessibleObjectException}. If they do throw, those classes cache
 * {@code initialized=false} for the JVM lifetime and every subsequent
 * {@code WindowsAWTWGLGraphicsConfigurationFactory.chooseGraphicsConfigurationImpl}
 * call throws {@code "Unable to determine GraphicsConfiguration: ... handle
 * 0x0, NullToolkitLock"}.
 *
 * <p>The canonical fix is the JVM arg
 * {@code --add-opens java.desktop/sun.awt=ALL-UNNAMED}, but we can't control
 * JVM args in a Minecraft mod. This class does it at runtime.
 *
 * <h2>Why the "obvious" Unsafe-override trick doesn't work on Java 21</h2>
 *
 * The classic approach flips the package-private {@code override} boolean on
 * {@code AccessibleObject} via {@code Unsafe.putBoolean} (to bypass the
 * {@code setAccessible} check) and then calls
 * {@code jdk.internal.module.Modules.addOpens}. This breaks in Java 21:
 * {@code AccessibleObject.class.getDeclaredField("override")} throws
 * {@code NoSuchFieldException} because the reflection filter
 * ({@code jdk.internal.reflect.Reflection.registerFieldsToFilter}) hides the
 * field from {@code getDeclaredFields} / {@code getDeclaredField}.
 *
 * <h2>What works on Java 21</h2>
 *
 * {@code MethodHandles.Lookup.IMPL_LOOKUP} — a private static field holding a
 * "trusted" {@code Lookup} with mode bits {@code 127} (all access) — is still
 * visible to {@code getDeclaredField} on Java 21; it's only
 * {@code setAccessible(true)} that the filter blocks (because
 * {@code java.base/java.lang.invoke} isn't open to unnamed modules).
 * {@code sun.misc.Unsafe} can bypass {@code setAccessible} entirely by
 * computing the field's offset and reading it via
 * {@code getObject(staticFieldBase, staticFieldOffset)}. That yields the
 * trusted lookup, from which we resolve
 * {@code jdk.internal.module.Modules.addOpens(Module, String, Module)} as a
 * {@code MethodHandle} and call it directly — no {@code setAccessible}
 * anywhere, so no filter, no {@code InaccessibleObjectException}.
 */
public final class DesktopModuleOpener {

    private static final Logger LOG = LoggerFactory.getLogger(DesktopModuleOpener.class);
    private static volatile boolean done;

    private DesktopModuleOpener() {}

    public static synchronized void openForJogl() {
        if (done) return;
        done = true;
        try {
            MethodHandles.Lookup trusted = acquireTrustedLookup();

            Class<?> modulesClass = Class.forName("jdk.internal.module.Modules");
            MethodHandle addOpens = trusted.findStatic(modulesClass, "addOpens",
                    MethodType.methodType(void.class, Module.class, String.class, Module.class));

            Module javaDesktop = java.awt.Toolkit.class.getModule();
            // JOGL classes are bundled into this mod jar via the fat-jar
            // merge in build.gradle. In NeoForge they end up in the
            // evanscomputermod named module.
            Module joglModule = com.jogamp.opengl.awt.GLCanvas.class.getModule();

            String[] packages = {
                    "sun.awt",
                    "sun.awt.windows",
                    "sun.awt.X11",
                    "sun.java2d",
            };
            int opened = 0;
            for (String pkg : packages) {
                // Some packages only exist on specific OSes; skip what's
                // not present in java.desktop on this platform.
                if (!javaDesktop.getPackages().contains(pkg)) continue;
                try {
                    addOpens.invoke(javaDesktop, pkg, joglModule);
                    opened++;
                } catch (Throwable t) {
                    LOG.warn("failed to add-open java.desktop/{} to {}: {}",
                            pkg, joglModule, t.toString());
                }
            }
            LOG.info("DesktopModuleOpener: opened {} sun.awt.* packages to JOGL module {}",
                    opened, joglModule.getName() == null ? "<unnamed>" : joglModule.getName());

            // Self-test: repeat the exact reflection JOGL will do. If this
            // fails, the whole opener approach is broken and the browser
            // will fail later with the original NullToolkitLock error.
            verifyJoglReflectionWorks();
        } catch (Throwable t) {
            LOG.error("DesktopModuleOpener failed — GLCanvas will fail with "
                    + "NullToolkitLock. Workaround: launch with "
                    + "--add-opens=java.desktop/sun.awt=ALL-UNNAMED "
                    + "--add-opens=java.desktop/sun.awt.windows=ALL-UNNAMED "
                    + "--add-opens=java.desktop/sun.awt.X11=ALL-UNNAMED "
                    + "--add-opens=java.desktop/sun.java2d=ALL-UNNAMED", t);
        }
    }

    /**
     * Obtain a {@link MethodHandles.Lookup} with all access modes (TRUSTED,
     * 127). Reads {@code MethodHandles.Lookup.IMPL_LOOKUP} via
     * {@code sun.misc.Unsafe} without going through
     * {@code Field.setAccessible}, which would be rejected on Java 21+
     * because {@code java.base/java.lang.invoke} isn't open to us.
     */
    private static MethodHandles.Lookup acquireTrustedLookup() throws Exception {
        Class<?> unsafeClass = Class.forName("sun.misc.Unsafe");
        Field theUnsafeField = unsafeClass.getDeclaredField("theUnsafe");
        theUnsafeField.setAccessible(true);
        Object unsafe = theUnsafeField.get(null);

        Method staticFieldOffset = unsafeClass.getMethod("staticFieldOffset", Field.class);
        Method staticFieldBase = unsafeClass.getMethod("staticFieldBase", Field.class);
        Method getObject = unsafeClass.getMethod("getObject", Object.class, long.class);

        Field implLookupField = MethodHandles.Lookup.class.getDeclaredField("IMPL_LOOKUP");
        long offset = (long) staticFieldOffset.invoke(unsafe, implLookupField);
        Object base = staticFieldBase.invoke(unsafe, implLookupField);
        return (MethodHandles.Lookup) getObject.invoke(unsafe, base, offset);
    }

    private static void verifyJoglReflectionWorks() {
        // Pick a class JOGL will actually reflect on, per platform.
        String platformClass = null;
        String platformMethod = null;
        String os = System.getProperty("os.name", "").toLowerCase();
        if (os.contains("win")) {
            platformClass = "sun.awt.Win32GraphicsConfig";
            platformMethod = "getVisual";
        } else if (os.contains("linux") || os.contains("nix") || os.contains("nux")) {
            platformClass = "sun.awt.X11GraphicsDevice";
            platformMethod = "getScreen";
        } else if (os.contains("mac")) {
            platformClass = "sun.awt.CGraphicsDevice";
            platformMethod = "getScreen";
        }
        if (platformClass == null) return;
        try {
            Class<?> cls = Class.forName(platformClass);
            Method m = cls.getDeclaredMethod(platformMethod);
            m.setAccessible(true);
            LOG.info("DesktopModuleOpener: self-test OK — {}.{} is reflectively accessible",
                    platformClass, platformMethod);
        } catch (Throwable t) {
            LOG.error("DesktopModuleOpener: self-test FAILED on {}.{} — {}",
                    platformClass, platformMethod, t.toString());
        }
    }
}
