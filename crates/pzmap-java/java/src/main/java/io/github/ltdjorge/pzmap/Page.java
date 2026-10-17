package io.github.ltdjorge.pzmap;

import java.util.List;

/**
 * One page of a {@link TexturePack}: a PNG image with many textures packed into it.
 *
 * <p>The image stays in native memory until {@link #png()} copies it out. So listing pages is
 * cheap, but {@link #png()} needs the pack to still be open.
 */
public final class Page {

    private final TexturePack pack;
    private final int index;
    private final String name;
    private final boolean hasAlpha;
    private final List<Texture> textures;

    Page(TexturePack pack, int index, String name, boolean hasAlpha, List<Texture> textures) {
        this.pack = pack;
        this.index = index;
        this.name = name;
        this.hasAlpha = hasAlpha;
        this.textures = textures;
    }

    /** {@return the page name stored in the pack} */
    public String name() {
        return name;
    }

    /** {@return whether the pack marks this page's image as having an alpha channel} */
    public boolean hasAlpha() {
        return hasAlpha;
    }

    /**
     * Returns every texture packed into this page.
     *
     * @return an unmodifiable list of textures, in file order
     */
    public List<Texture> textures() {
        return textures;
    }

    /**
     * Returns the page image as PNG bytes.
     *
     * <p>Each call copies the image out of native memory into a new array. This library does not
     * decode the image.
     *
     * @return the PNG file bytes
     * @throws IllegalStateException if the pack has been closed
     */
    public byte[] png() {
        return pack.pagePng(index);
    }

    @Override
    public String toString() {
        return "Page[name=" + name + ", textures=" + textures.size() + "]";
    }
}
