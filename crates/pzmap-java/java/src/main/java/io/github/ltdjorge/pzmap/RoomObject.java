package io.github.ltdjorge.pzmap;

/**
 * An object placed in a room, in world coordinates.
 *
 * <p>It is not called {@code Object} because that name would hide {@link java.lang.Object} in any
 * file that imports this package with a wildcard.
 *
 * @param type the object's type number as stored in the file, which this library does not
 *     interpret
 * @param x the object's world X coordinate
 * @param y the object's world Y coordinate
 */
public record RoomObject(int type, int x, int y) {}
