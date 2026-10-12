package com.example.evanscomputermod.radio.cellular;

/**
 * A cellular modem as the computer's kernel sees it through the {@code cell_*}
 * host functions ({@code wwan0}; abi/host-abi.toml). The LTE and mmWave modem
 * modules implement it. Version-neutral so {@code ComputerInstance} can look
 * for it on every Minecraft version; only 1.21.1 has an implementation.
 */
public interface CellRadio {

    /** True while the modem is installed and live in the world. */
    boolean cellActive();

    /** The modem engine behind this radio. */
    ModemLink modem();

    /** Called with a notifier that raises the computer's network IRQ when frames arrive (null unbinds). */
    default void bindInterrupt(Runnable irq) {
        modem().setRxNotify(irq);
    }
}
