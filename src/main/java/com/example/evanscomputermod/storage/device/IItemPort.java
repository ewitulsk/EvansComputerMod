package com.example.evanscomputermod.storage.device;

//? if <=1.21.1 {

import java.util.UUID;

/** An Encoder or Decoder a computer can reach (next to it or on its wired bus). */
public interface IItemPort {

    UUID deviceId();

    /** {@code "encoder"} or {@code "decoder"}. */
    String kind();

    boolean isLive();
}
//?}
