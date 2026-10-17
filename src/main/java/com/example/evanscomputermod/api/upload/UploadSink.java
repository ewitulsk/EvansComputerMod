package com.example.evanscomputermod.api.upload;

/**
 * One upload into one computer, as the {@link UploadBackend} sees it. Calls
 * come in order: {@link #begin}, then {@link #write} any number of times,
 * then {@link #finish}; {@link #fail} ends it at any point. Each returns
 * {@link UploadStatus#OK} or the error that ended the upload (the backend
 * then stops sending and tells the player). Server thread.
 */
public interface UploadSink {
    /** Names this upload in packets; unique while the server runs. */
    int ticket();

    /** Where the file goes, as the computer sees it (for example {@code /home}). */
    String destination();

    /** What to call the computer to the player ("Terminal", "ePhone"). */
    String computerKind();

    /** The player picked {@code fileName} of {@code size} bytes. */
    int begin(String fileName, long size);

    /** The next bytes of the file. */
    int write(byte[] data, int offset, int length);

    /** Every byte was sent: the file appears on the computer. */
    int finish();

    /** The upload ended without a file, with an {@link UploadStatus} error (cancelled, too big, ...). */
    void fail(int status);

    /** Where the file landed on the computer (for example {@code /home/photo.png}) once it is done, else null. */
    String path();

    /** True once the upload is done or failed (later calls are ignored). */
    boolean closed();
}
