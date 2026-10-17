package io.github.ltdjorge.pzmap;

import java.io.IOException;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;

/**
 * A parsed {@code .pack} texture atlas: a list of PNG pages, and where each texture sits on them.
 *
 * <p>This library does not decode the images. {@link Page#png()} returns the PNG bytes exactly as
 * they are stored in the file.
 *
 * <p>The page images are held in native memory, outside the Java heap. A pack from the base game
 * holds hundreds of megabytes, so close it when you are done, ideally with try-with-resources:
 *
 * <pre>{@code
 * try (TexturePack pack = TexturePack.parse(Path.of("Tiles2x.pack"))) {
 *     for (Page page : pack.pages()) {
 *         Files.write(Path.of(page.name() + ".png"), page.png());
 *     }
 * }
 * }</pre>
 *
 * <p>After {@link #close()}, {@link Page#png()} throws {@link IllegalStateException}. Everything
 * else keeps working, because page names and texture positions are copied to Java when the pack
 * is parsed. See the {@linkplain io.github.ltdjorge.pzmap package documentation} for the rest of
 * the rules on closing and threads.
 */
public final class TexturePack implements AutoCloseable {

    /** The number of ints per texture in the native rectangle array: x, y, w, h, ox, oy, ow, oh. */
    private static final int TEXTURE_STRIDE = 8;

    static {
        NativeLoader.ensureLoaded();
    }

    private final NativeHandle handle;
    private final List<Page> pages;
    private final int textureCount;

    private TexturePack(long pointer) {
        this.handle = new NativeHandle(pointer, "TexturePack", TexturePack::nativeFree);

        // Copy every page's name and textures in one call. The images stay in native memory until
        // Page.png() asks for one.
        Object[] flat = handle.call(TexturePack::nativePages);
        String[] names = (String[]) flat[0];
        int[] hasAlpha = (int[]) flat[1];
        int[] textureStarts = (int[]) flat[2];
        String[] textureNames = (String[]) flat[3];
        int[] rects = (int[]) flat[4];

        List<Page> pages = new ArrayList<>(names.length);
        for (int page = 0; page < names.length; page++) {
            List<Texture> textures = new ArrayList<>();
            for (int i = textureStarts[page]; i < textureStarts[page + 1]; i++) {
                int base = i * TEXTURE_STRIDE;
                textures.add(
                        new Texture(
                                textureNames[i],
                                rects[base],
                                rects[base + 1],
                                rects[base + 2],
                                rects[base + 3],
                                rects[base + 4],
                                rects[base + 5],
                                rects[base + 6],
                                rects[base + 7]));
            }
            pages.add(new Page(this, page, names[page], hasAlpha[page] != 0, List.copyOf(textures)));
        }
        this.pages = List.copyOf(pages);
        this.textureCount = textureNames.length;
    }

    /**
     * Reads and parses a {@code .pack} file.
     *
     * @param path the {@code .pack} file
     * @return the parsed pack, which the caller must close
     * @throws PzmapParseException if the file is malformed or uses a pack version this library
     *     does not support
     * @throws IOException if the file cannot be read
     */
    public static TexturePack parse(Path path) throws IOException {
        return new TexturePack(nativeParsePath(path.toString()));
    }

    /**
     * Parses a {@code .pack} from a byte array.
     *
     * <p>The native library copies {@code data}, so the array can be reused afterwards.
     *
     * @param data the contents of a {@code .pack} file
     * @return the parsed pack, which the caller must close
     * @throws PzmapParseException if the data is malformed or uses a pack version this library
     *     does not support
     */
    public static TexturePack parse(byte[] data) throws PzmapParseException {
        return new TexturePack(nativeParseBytes(data));
    }

    /** {@return the number of pages in the pack} */
    public int pageCount() {
        return pages.size();
    }

    /** {@return the total number of textures across all pages} */
    public int textureCount() {
        return textureCount;
    }

    /**
     * Returns every page in the pack, in file order.
     *
     * <p>This does not copy any images. Call {@link Page#png()} to copy one.
     *
     * @return an unmodifiable list of pages, which stays usable after the pack is closed
     */
    public List<Page> pages() {
        return pages;
    }

    /** {@return a copy of one page's PNG bytes} {@link Page#png()} calls this. */
    byte[] pagePng(int page) {
        return handle.call(pointer -> nativePagePng(pointer, page));
    }

    /** {@return whether {@link #close()} has been called} */
    public boolean isClosed() {
        return handle.isClosed();
    }

    /**
     * Frees the native memory behind this pack.
     *
     * <p>Waits for calls in progress on other threads to finish first. Calling this again does
     * nothing.
     */
    @Override
    public void close() {
        handle.close();
    }

    @Override
    public String toString() {
        return "TexturePack[pages=" + pages.size() + ", textures=" + textureCount + "]";
    }

    private static native long nativeParsePath(String path);

    private static native long nativeParseBytes(byte[] data);

    private static native void nativeFree(long handle);

    private static native Object[] nativePages(long handle);

    private static native byte[] nativePagePng(long handle, int page);
}
