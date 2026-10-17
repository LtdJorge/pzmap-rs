package io.github.ltdjorge.pzmap;

/**
 * One texture (sprite) on a {@link Page}.
 *
 * <p>The packer cuts the transparent border off each sprite before packing it. {@code x, y, w, h}
 * is the trimmed rectangle on the page image. {@code ow} and {@code oh} are the size of the
 * original sprite before trimming, and {@code ox, oy} is where the trimmed rectangle sat inside it.
 *
 * <p>The values are exactly as stored in the file. This class does not compute where to draw the
 * sprite, because that depends on the renderer.
 *
 * @param name the texture name
 * @param x the X position of the trimmed rectangle on the page image
 * @param y the Y position of the trimmed rectangle on the page image
 * @param w the width of the trimmed rectangle
 * @param h the height of the trimmed rectangle
 * @param ox the X offset of the trimmed rectangle within the original sprite
 * @param oy the Y offset of the trimmed rectangle within the original sprite
 * @param ow the width of the original sprite
 * @param oh the height of the original sprite
 */
public record Texture(String name, int x, int y, int w, int h, int ox, int oy, int ow, int oh) {}
