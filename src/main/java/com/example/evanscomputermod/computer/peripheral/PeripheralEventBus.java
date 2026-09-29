package com.example.evanscomputermod.computer.peripheral;

import org.jetbrains.annotations.Nullable;

import java.util.ArrayDeque;
import java.util.ArrayList;
import java.util.List;
import java.util.concurrent.CopyOnWriteArrayList;

/**
 * Peripheral events for one computer. Every program that has used the
 * {@code peripheral} API holds a {@link Subscriber}; each posted event is
 * copied into every subscriber's queue, so two programs waiting on events
 * both see it. Owned by the {@code ComputerInstance}, so subscriptions survive
 * the computer being carried onto a Sable sub-level.
 *
 * <p>Queues are bounded ({@link #QUEUE_CAPACITY}): when a program stops
 * reading, the oldest events are dropped.
 */
public final class PeripheralEventBus {

    public static final int QUEUE_CAPACITY = 256;

    /** An event as delivered: its name and the encoded LIST {@code [name, attachment, *args]}. */
    public record Event(String name, byte[] encoded) {}

    public static final class Subscriber {
        private final ArrayDeque<Event> queue = new ArrayDeque<>();
        private int dropped;

        /** Number of events dropped because the queue was full. */
        public synchronized int dropped() {
            return dropped;
        }
    }

    private final List<Subscriber> subscribers = new CopyOnWriteArrayList<>();

    public Subscriber subscribe() {
        Subscriber s = new Subscriber();
        subscribers.add(s);
        return s;
    }

    public void unsubscribe(@Nullable Subscriber s) {
        if (s == null) return;
        subscribers.remove(s);
        synchronized (s) {
            s.queue.clear();
            s.notifyAll();
        }
    }

    public boolean hasSubscribers() {
        return !subscribers.isEmpty();
    }

    /** Queue {@code (event, attachment, *args)} for every subscriber. Any thread. */
    public void post(String event, String attachment, Object... args) {
        if (subscribers.isEmpty()) return;
        List<Object> payload = new ArrayList<>(args.length + 2);
        payload.add(event);
        payload.add(attachment);
        java.util.Collections.addAll(payload, args);
        Event e = new Event(event, PeripheralValues.encode(payload));
        for (Subscriber s : subscribers) {
            synchronized (s) {
                if (s.queue.size() >= QUEUE_CAPACITY) {
                    s.queue.pollFirst();
                    s.dropped++;
                }
                s.queue.addLast(e);
                s.notifyAll();
            }
        }
    }

    /**
     * Take the next event named {@code filter} (any event if null), discarding
     * others ahead of it. Waits up to {@code timeoutMs} (0 = don't wait,
     * negative = forever).
     *
     * @return the event, or null on timeout
     * @throws InterruptedException if the program is killed while waiting
     */
    @Nullable
    public static Event await(Subscriber s, @Nullable String filter, long timeoutMs) throws InterruptedException {
        long deadline = timeoutMs < 0 ? Long.MAX_VALUE : System.currentTimeMillis() + timeoutMs;
        synchronized (s) {
            while (true) {
                Event e;
                while ((e = s.queue.pollFirst()) != null) {
                    if (filter == null || filter.equals(e.name())) return e;
                }
                long now = System.currentTimeMillis();
                if (now >= deadline) return null;
                // Wake at least every 250 ms so a kill that only sets a flag is noticed.
                s.wait(Math.min(250, deadline - now));
                if (Thread.currentThread().isInterrupted()) throw new InterruptedException();
            }
        }
    }
}
