package com.example.evanscomputermod.wasm.chicory;

import com.example.evanscomputermod.api.wasm.WasmMemory;
import com.dylibso.chicory.runtime.Memory;

/**
 * Thin pass-through to Chicory's {@link Memory} interface. Endianness is LE
 * across the board (WASM's only endianness; Chicory honors that).
 */
final class ChicoryMemory implements WasmMemory {

    private final Memory mem;

    ChicoryMemory(Memory mem) {
        this.mem = mem;
    }

    @Override public int size()  { return mem.pages() * Memory.PAGE_SIZE; }
    @Override public int pages() { return mem.pages(); }
    @Override public int grow(int deltaPages) { return mem.grow(deltaPages); }

    @Override public byte readByte(int addr)         { return mem.read(addr); }
    @Override public void writeByte(int addr, byte v){ mem.writeByte(addr, v); }

    @Override public short readShort(int addr)         { return mem.readShort(addr); }
    @Override public void  writeShort(int addr, short v){ mem.writeShort(addr, v); }

    @Override public int  readInt(int addr)        { return mem.readInt(addr); }
    @Override public void writeInt(int addr, int v){ mem.writeI32(addr, v); }

    @Override public long readLong(int addr)         { return mem.readLong(addr); }
    @Override public void writeLong(int addr, long v){ mem.writeLong(addr, v); }

    @Override public float readFloat(int addr)         { return mem.readFloat(addr); }
    @Override public void  writeFloat(int addr, float v){ mem.writeF32(addr, v); }

    @Override public double readDouble(int addr)         { return mem.readDouble(addr); }
    @Override public void   writeDouble(int addr, double v){ mem.writeF64(addr, v); }

    @Override public byte[] readBytes(int addr, int len) { return mem.readBytes(addr, len); }
    @Override public void   writeBytes(int addr, byte[] src, int srcOff, int len) {
        mem.write(addr, src, srcOff, len);
    }
}
