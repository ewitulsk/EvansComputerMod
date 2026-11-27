package com.example.evanscomputermod.api.repo;

/**
 * One request a {@link RepoLink} is making. Polled by the computer that asked
 * ({@code repo_poll} / {@code repo_read}); every method may be called from any thread.
 */
public interface RepoRequest {
    int CONNECTING = 0;
    int ANSWERED = 1;
    int DONE = 2;
    int FAILED = -14;

    /** {@link #CONNECTING}, {@link #ANSWERED} (the body is arriving), {@link #DONE} or {@link #FAILED}. */
    int state();

    /** The HTTP status, once answered. */
    int status();

    /** Body bytes received so far. */
    long received();

    /** The body's length, or -1 when the repository did not say. */
    long total();

    /** Copy up to {@code buf.length} unread body bytes; 0 when none are waiting. */
    int read(byte[] buf);

    /** Why it failed (for {@link #FAILED}). */
    String error();

    /** Stop and forget. */
    void close();
}
