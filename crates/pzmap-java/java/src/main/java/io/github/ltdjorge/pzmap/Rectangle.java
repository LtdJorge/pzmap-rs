package io.github.ltdjorge.pzmap;

/**
 * An axis-aligned rectangle of squares, in world coordinates.
 *
 * @param x the world X of the rectangle's origin
 * @param y the world Y of the rectangle's origin
 * @param width the width in squares
 * @param height the height in squares
 */
public record Rectangle(int x, int y, int width, int height) {}
