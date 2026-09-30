package com.example.evanscomputermod.computer.wasi;

import static org.junit.jupiter.api.Assertions.*;

import java.util.concurrent.CountDownLatch;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicBoolean;
import java.util.concurrent.atomic.AtomicReference;
import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.Timeout;

class FdTableCancellationTest {
  private static final class Resource implements WasiFileDescriptor {
    final AtomicBoolean closed = new AtomicBoolean();

    public int read(byte[] b, int o, int l) {
      return -1;
    }

    public int write(byte[] b, int o, int l) {
      return -1;
    }

    public boolean isReadable() {
      return false;
    }

    public boolean isWritable() {
      return false;
    }

    public void close() {
      closed.set(true);
    }
  }

  @Test
  @Timeout(5)
  void cancellationDuringSocketCloseStillReleasesOtherDescriptors() throws Exception {
    var bridge = new NetIpcBridge();
    var waiting = new CountDownLatch(1);
    bridge.setWakeListener(waiting::countDown);
    var table = new FdTable();
    var other = new Resource();
    table.insertAt(4, new SocketFd(17, 42, bridge));
    table.insertAt(5, other);
    var uncaught = new AtomicReference<Throwable>();
    var interrupted = new AtomicBoolean();
    var child =
        new Thread(
            () -> {
              table.closeAll();
              interrupted.set(Thread.currentThread().isInterrupted());
            });
    child.setUncaughtExceptionHandler((t, e) -> uncaught.set(e));
    child.start();
    try {
      assertTrue(waiting.await(2, TimeUnit.SECONDS));
      child.interrupt();
      child.join(1000);
      assertFalse(child.isAlive());
      assertNull(uncaught.get());
      assertTrue(interrupted.get());
      assertTrue(other.closed.get());
      assertFalse(table.contains(4));
      assertFalse(table.contains(5));
      bridge.cancelPending(42);
      assertFalse(bridge.hasPending());
    } finally {
      child.interrupt();
      child.join(1000);
      bridge.cancelPending(42);
    }
  }

  @Test
  void alreadyInterruptedSocketCloseDoesNotAbortCleanup() throws Exception {
    var table = new FdTable();
    var other = new Resource();
    table.insertAt(4, new SocketFd(17, 42, new NetIpcBridge()));
    table.insertAt(5, other);
    var uncaught = new AtomicReference<Throwable>();
    var child =
        new Thread(
            () -> {
              Thread.currentThread().interrupt();
              table.closeAll();
            });
    child.setUncaughtExceptionHandler((t, e) -> uncaught.set(e));
    child.start();
    child.join(1000);
    assertFalse(child.isAlive());
    assertNull(uncaught.get());
    assertTrue(other.closed.get());
  }

  @Test
  void unexpectedCloseFailureIsNotSilenced() {
    var table = new FdTable();
    table.insertAt(
        4,
        new SocketFd(
            17,
            42,
            new NetIpcBridge() {
              @Override
              public Result call(int session, int syscall, byte[] args) {
                throw new IllegalStateException("unexpected IPC failure");
              }
            }));
    assertThrows(IllegalStateException.class, table::closeAll);
  }
}
