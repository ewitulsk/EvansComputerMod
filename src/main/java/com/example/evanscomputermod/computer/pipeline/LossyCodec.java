package com.example.evanscomputermod.computer.pipeline;

import com.example.evanscomputermod.computer.TerminalDisplay;

import javax.imageio.IIOImage;
import javax.imageio.ImageIO;
import javax.imageio.ImageReader;
import javax.imageio.ImageWriteParam;
import javax.imageio.ImageWriter;
import javax.imageio.stream.ImageInputStream;
import javax.imageio.stream.ImageOutputStream;
import java.awt.image.BufferedImage;
import java.awt.image.DataBufferInt;
import java.io.ByteArrayInputStream;
import java.io.ByteArrayOutputStream;
import java.io.IOException;
import java.util.Iterator;

/**
 * Lossy frames for moving pictures (DISPLAY_PIPELINE_SPEC.md Part D): a whole
 * graphics plane as one baseline JPEG, with the JDK's own {@code javax.imageio}
 * codec on both ends (nothing native, nothing shipped by the mod).
 *
 * <p>Frames are whole and independent, so a viewer that can't keep up can skip
 * one, and a lost ack never leaves a picture built on a frame the client
 * didn't get. The quality ladder is {@link #RUNGS}: 0.85, 0.75, 0.6, 0.45,
 * and the last rung also halves the resolution (the client scales it back up).
 *
 * <p>Pixel formats: RGB565 and RGBA8888 (alpha is dropped: lossy frames are
 * opaque). Palette displays (INDEXED8) stay exact: converting JPEG colours
 * back to a palette would need quantising on the client, which costs more
 * than it saves at the sizes palette programs use.
 *
 * <p>One instance per thread (the hubs own one each; clients use
 * {@link #forThisThread()}).
 */
public final class LossyCodec {
    /** Quality per rung; the last one is at half resolution. */
    public static final float[] RUNGS = {0.85f, 0.75f, 0.6f, 0.45f, 0.45f};
    public static final int HALF_RES_RUNG = 4;

    private static final ThreadLocal<LossyCodec> LOCAL = ThreadLocal.withInitial(LossyCodec::new);

    public static LossyCodec forThisThread() {
        return LOCAL.get();
    }

    private BufferedImage image;
    private ImageWriter writer;
    private ImageReader reader;
    private final ByteArrayOutputStream out = new ByteArrayOutputStream(64 * 1024);
    private int[] scratch = new int[0];

    /** Can lossy frames carry this pixel format? */
    public static boolean supports(int pixelFormat) {
        return pixelFormat == TerminalDisplay.PIXEL_FORMAT_RGB565 || pixelFormat == TerminalDisplay.PIXEL_FORMAT_RGBA8888;
    }

    private BufferedImage image(int w, int h) {
        if (image == null || image.getWidth() != w || image.getHeight() != h) {
            image = new BufferedImage(w, h, BufferedImage.TYPE_INT_RGB);
        }
        return image;
    }

    /**
     * Encode a {@code w x h} plane of {@code pixels} in {@code format} at
     * {@code rung} of {@link #RUNGS}. Returns the JPEG bytes.
     */
    public byte[] encode(byte[] pixels, int w, int h, int format, int rung) throws IOException {
        int r = Math.max(0, Math.min(RUNGS.length - 1, rung));
        boolean half = r == HALF_RES_RUNG && w >= 32 && h >= 32;
        int ow = half ? (w + 1) / 2 : w, oh = half ? (h + 1) / 2 : h;
        BufferedImage img = image(ow, oh);
        int[] rgb = ((DataBufferInt) img.getRaster().getDataBuffer()).getData();
        if (half) {
            if (scratch.length < w * h) scratch = new int[w * h];
            toRgb(pixels, w, h, format, scratch);
            for (int y = 0; y < oh; y++) {
                int y0 = y * 2, y1 = Math.min(h - 1, y0 + 1);
                for (int x = 0; x < ow; x++) {
                    int x0 = x * 2, x1 = Math.min(w - 1, x0 + 1);
                    rgb[y * ow + x] = average(scratch[y0 * w + x0], scratch[y0 * w + x1], scratch[y1 * w + x0], scratch[y1 * w + x1]);
                }
            }
        } else {
            toRgb(pixels, w, h, format, rgb);
        }
        if (writer == null) {
            Iterator<ImageWriter> it = ImageIO.getImageWritersByFormatName("jpeg");
            if (!it.hasNext()) throw new IOException("no JPEG writer in this JDK");
            writer = it.next();
        }
        ImageWriteParam p = writer.getDefaultWriteParam();
        p.setCompressionMode(ImageWriteParam.MODE_EXPLICIT);
        p.setCompressionQuality(RUNGS[r]);
        out.reset();
        try (ImageOutputStream ios = ImageIO.createImageOutputStream(out)) {
            writer.setOutput(ios);
            writer.write(null, new IIOImage(img, null, null), p);
        } finally {
            writer.setOutput(null);
        }
        return out.toByteArray();
    }

    private static int average(int a, int b, int c, int d) {
        int r = (((a >> 16) & 0xFF) + ((b >> 16) & 0xFF) + ((c >> 16) & 0xFF) + ((d >> 16) & 0xFF) + 2) >> 2;
        int g = (((a >> 8) & 0xFF) + ((b >> 8) & 0xFF) + ((c >> 8) & 0xFF) + ((d >> 8) & 0xFF) + 2) >> 2;
        int bl = ((a & 0xFF) + (b & 0xFF) + (c & 0xFF) + (d & 0xFF) + 2) >> 2;
        return r << 16 | g << 8 | bl;
    }

    /** A plane in {@code format} as 0xRRGGBB ints. */
    static void toRgb(byte[] px, int w, int h, int format, int[] rgb) {
        int n = w * h;
        if (format == TerminalDisplay.PIXEL_FORMAT_RGB565) {
            for (int i = 0; i < n; i++) {
                int v = (px[i * 2] & 0xFF) | (px[i * 2 + 1] & 0xFF) << 8;
                int r = (v >> 11) & 0x1F, g = (v >> 5) & 0x3F, b = v & 0x1F;
                rgb[i] = (r << 3 | r >> 2) << 16 | (g << 2 | g >> 4) << 8 | (b << 3 | b >> 2);
            }
        } else {
            for (int i = 0; i < n; i++) {
                rgb[i] = (px[i * 4] & 0xFF) << 16 | (px[i * 4 + 1] & 0xFF) << 8 | (px[i * 4 + 2] & 0xFF);
            }
        }
    }

    /**
     * Decode a JPEG frame into {@code dst}, a {@code w x h} plane in
     * {@code format}. A half-resolution frame is scaled back up. Returns false
     * if the data isn't a JPEG of a size that fits.
     */
    public boolean decode(byte[] jpeg, int w, int h, int format, byte[] dst) {
        if (!supports(format) || dst == null || dst.length < w * h * TerminalDisplay.bytesPerPixel(format)) return false;
        BufferedImage img;
        try {
            if (reader == null) {
                Iterator<ImageReader> it = ImageIO.getImageReadersByFormatName("jpeg");
                if (!it.hasNext()) return false;
                reader = it.next();
            }
            try (ImageInputStream iis = ImageIO.createImageInputStream(new ByteArrayInputStream(jpeg))) {
                reader.setInput(iis, true, true);
                img = reader.read(0);
            } finally {
                reader.setInput(null);
            }
        } catch (IOException | RuntimeException e) {
            return false;
        }
        int iw = img.getWidth(), ih = img.getHeight();
        boolean half = iw == (w + 1) / 2 && ih == (h + 1) / 2 && (iw != w || ih != h);
        if (!half && (iw != w || ih != h)) return false;
        int[] src = new int[iw * ih];
        img.getRGB(0, 0, iw, ih, src, 0, iw);
        boolean rgb565 = format == TerminalDisplay.PIXEL_FORMAT_RGB565;
        for (int y = 0; y < h; y++) {
            int sy = half ? Math.min(ih - 1, y / 2) : y;
            for (int x = 0; x < w; x++) {
                int sx = half ? Math.min(iw - 1, x / 2) : x;
                int c = src[sy * iw + sx];
                int i = y * w + x;
                if (rgb565) {
                    int v = ((c >> 19) & 0x1F) << 11 | ((c >> 10) & 0x3F) << 5 | ((c >> 3) & 0x1F);
                    dst[i * 2] = (byte) v;
                    dst[i * 2 + 1] = (byte) (v >> 8);
                } else {
                    dst[i * 4] = (byte) (c >> 16);
                    dst[i * 4 + 1] = (byte) (c >> 8);
                    dst[i * 4 + 2] = (byte) c;
                    dst[i * 4 + 3] = (byte) 0xFF;
                }
            }
        }
        return true;
    }

    /** Decode into premultiplied ARGB ints ({@code w x h}, opaque), for canvases and pixel surfaces. */
    public boolean decodeArgb(byte[] jpeg, int w, int h, int[] dst) {
        byte[] tmp = new byte[w * h * 4];
        if (!decode(jpeg, w, h, TerminalDisplay.PIXEL_FORMAT_RGBA8888, tmp)) return false;
        for (int i = 0; i < w * h; i++) {
            dst[i] = 0xFF000000 | (tmp[i * 4] & 0xFF) << 16 | (tmp[i * 4 + 1] & 0xFF) << 8 | (tmp[i * 4 + 2] & 0xFF);
        }
        return true;
    }

    /** Encode premultiplied ARGB ints (alpha dropped). */
    public byte[] encodeArgb(int[] argb, int w, int h, int rung) throws IOException {
        byte[] tmp = new byte[w * h * 4];
        for (int i = 0; i < w * h; i++) {
            int c = argb[i];
            tmp[i * 4] = (byte) (c >> 16);
            tmp[i * 4 + 1] = (byte) (c >> 8);
            tmp[i * 4 + 2] = (byte) c;
            tmp[i * 4 + 3] = (byte) 0xFF;
        }
        return encode(tmp, w, h, TerminalDisplay.PIXEL_FORMAT_RGBA8888, rung);
    }
}
