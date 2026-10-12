package com.example.evanscomputermod.computer.ui;

import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Map;

/** A {@link CompositorHost} for tests: manual clock, scripted spawns, recorded frames. */
final class UiTestHost implements CompositorHost {
    long now;
    final Map<Integer, String> names = new HashMap<>();
    final List<String> spawned = new ArrayList<>();
    /** Arguments of every spawn, in order (the picker's token). */
    final List<List<String>> spawnArgs = new ArrayList<>();
    /** P4 hooks: filePicked "pid path write", openWith "path app always". */
    final List<String> picked = new ArrayList<>();
    final List<String> opened = new ArrayList<>();
    final List<Integer> killed = new ArrayList<>();
    final List<String> logs = new ArrayList<>();
    /** Damage of every present, in order. */
    final List<List<Box>> presents = new ArrayList<>();
    /** Also told about every present (client rendering tests stream the frames as tiles). */
    interface Presenter {
        void present(byte[] rgba, int w, int h, List<Box> damage);
    }

    Presenter onPresent;
    /** What spawn returns next (-1: the program can't run). */
    int nextPid = -1;

    @Override
    public int spawn(String program, List<String> args) {
        spawned.add(program);
        spawnArgs.add(List.copyOf(args));
        int pid = nextPid;
        if (pid > 0) {
            names.put(pid, program);
            nextPid++;
        }
        return pid;
    }

    @Override
    public void filePicked(int pid, String path, boolean write) {
        picked.add(pid + " " + path + " " + write);
    }

    @Override
    public void openWith(String path, String appId, boolean always) {
        opened.add(path + " " + appId + " " + always);
    }

    @Override
    public void kill(int pid) {
        killed.add(pid);
    }

    @Override
    public String appId(int pid) {
        return names.getOrDefault(pid, "pid-" + pid);
    }

    @Override
    public void present(byte[] rgba, int w, int h, List<Box> damage) {
        presents.add(List.copyOf(damage));
        if (onPresent != null) onPresent.present(rgba, w, h, damage);
    }

    @Override
    public void requestTick(long delayNanos) {}

    @Override
    public long nowMs() {
        return now;
    }

    @Override
    public void log(String msg) {
        logs.add(msg);
    }
}
