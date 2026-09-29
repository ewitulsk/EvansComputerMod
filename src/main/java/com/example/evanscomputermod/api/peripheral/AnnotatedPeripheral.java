package com.example.evanscomputermod.api.peripheral;

import org.jetbrains.annotations.Nullable;

import java.util.Set;

/**
 * Base class for peripherals whose callable methods are marked with
 * {@link PeripheralMethod}:
 * <pre>
 * public class LampPeripheral extends AnnotatedPeripheral {
 *     public String getType() { return "lamp"; }
 *
 *     {@literal @}PeripheralMethod(description = "Turn the lamp on or off")
 *     public void setLit(boolean lit) { ... }        // callable as lamp.set_lit(True)
 * }
 * </pre>
 */
public abstract class AnnotatedPeripheral implements IPeripheral {

    private final PeripheralMethods methods = PeripheralMethods.of(getClass());

    @Override
    public Set<String> getMethodNames() {
        return methods.names();
    }

    @Override
    @Nullable
    public Object callMethod(IComputerAccess computer, String method, Object[] arguments) throws PeripheralException {
        return methods.call(this, computer, method, arguments);
    }

    @Override
    public boolean runsOnMainThread(String method) {
        return methods.isMainThread(method);
    }
}
