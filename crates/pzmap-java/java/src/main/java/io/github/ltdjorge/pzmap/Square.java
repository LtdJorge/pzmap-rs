package io.github.ltdjorge.pzmap;

import java.util.Arrays;

/**
 * One non-empty square of a {@link Cell}: where it is, and which tiles are on it.
 *
 * <p>{@link #tiles()} returns the record's own array, not a copy, so do not modify it.
 * {@code equals} and {@code hashCode} compare the array's contents.
 *
 * @param block the block index within the cell, equal to
 *     {@code blockX * Build.cellSizeInBlocks() + blockY}
 * @param layer the level the square is on
 * @param x the square's X position within its block
 * @param y the square's Y position within its block
 * @param room the room id stored by the map editor, or a negative value for none. It is not an
 *     index into {@link LotHeader#rooms()}. To find a square's room, test the square against the
 *     room rectangles instead.
 * @param tiles the tile indices into {@link LotHeader#strings()}, in draw order, bottom first
 */
public record Square(int block, int layer, int x, int y, int room, int[] tiles) {

    @Override
    public boolean equals(Object other) {
        return other instanceof Square square
                && block == square.block
                && layer == square.layer
                && x == square.x
                && y == square.y
                && room == square.room
                && Arrays.equals(tiles, square.tiles);
    }

    @Override
    public int hashCode() {
        int hash = Integer.hashCode(block);
        hash = 31 * hash + Integer.hashCode(layer);
        hash = 31 * hash + Integer.hashCode(x);
        hash = 31 * hash + Integer.hashCode(y);
        hash = 31 * hash + Integer.hashCode(room);
        return 31 * hash + Arrays.hashCode(tiles);
    }

    @Override
    public String toString() {
        return "Square[block=" + block + ", layer=" + layer + ", x=" + x + ", y=" + y + ", room="
                + room + ", tiles=" + Arrays.toString(tiles) + "]";
    }
}
