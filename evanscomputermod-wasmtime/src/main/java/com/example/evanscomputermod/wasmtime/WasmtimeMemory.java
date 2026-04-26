package com.example.evanscomputermod.wasmtime;

import com.example.evanscomputermod.api.wasm.WasmMemory;

import io.github.kawamuray.wasmtime.Memory;
import io.github.kawamuray.wasmtime.Store;

import java.nio.ByteBuffer;
import java.nio.ByteOrder;

/**
 * {@link WasmMemory} backed by wasmtime-java's {@link Memory}. Each access
 * fetches a fresh {@link ByteBuffer} view from the store and applies LE
 * byte order — wasmtime returns the memory as a direct byte buffer that
 * may move on grow.
 */
final class WasmtimeMemory implements WasmMemory {

    private final Store<Void> store;
    private final Memory memory;

    WasmtimeMemory(Store<Void> store, Memory memory) {
        this.store = store;
        this.memory = memory;
    }

    private ByteBuffer buf() {
        return memory.buffer(store).order(ByteOrder.LITTLE_ENDIAN);
    }

    @Override public int size()  { return buf().capacity(); }
    @Override public int pages() { return buf().capacity() / 65536; }

    @Override
    public int grow(int deltaPages) {
        int prev = pages();
        try {
            memory.grow(store, deltaPages);
            return prev;
        } catch (Throwable t) {
            return -1;
        }
    }

    @Override public byte readByte(int addr)         { return buf().get(addr); }
    @Override public void writeByte(int addr, byte v){ buf().put(addr, v); }

    @Override public short readShort(int addr)         { return buf().getShort(addr); }
    @Override public void  writeShort(int addr, short v){ buf().putShort(addr, v); }

    @Override public int  readInt(int addr)        { return buf().getInt(addr); }
    @Override public void writeInt(int addr, int v){ buf().putInt(addr, v); }

    @Override public long readLong(int addr)         { return buf().getLong(addr); }
    @Override public void writeLong(int addr, long v){ buf().putLong(addr, v); }

    @Override public float readFloat(int addr)         { return buf().getFloat(addr); }
    @Override public void  writeFloat(int addr, float v){ buf().putFloat(addr, v); }

    @Override public double readDouble(int addr)         { return buf().getDouble(addr); }
    @Override public void   writeDouble(int addr, double v){ buf().putDouble(addr, v); }

    @Override
    public byte[] readBytes(int addr, int len) {
        byte[] out = new byte[len];
        ByteBuffer b = buf();
        for (int i = 0; i < len; i++) out[i] = b.get(addr + i);
        return out;
    }

    @Override
    public void writeBytes(int addr, byte[] src, int srcOff, int len) {
        ByteBuffer b = buf();
        for (int i = 0; i < len; i++) b.put(addr + i, src[srcOff + i]);
    }
}
