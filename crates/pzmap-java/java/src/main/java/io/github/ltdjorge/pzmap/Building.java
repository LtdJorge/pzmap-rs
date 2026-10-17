package io.github.ltdjorge.pzmap;

import java.util.Arrays;

/**
 * A building in a {@link LotHeader}, described by the rooms it contains.
 *
 * <p>{@link #roomIndices()} returns the record's own array, not a copy, so do not modify it.
 * {@code equals} and {@code hashCode} compare the array's contents.
 *
 * @param roomIndices the indices of this building's rooms in {@link LotHeader#rooms()}
 */
public record Building(int[] roomIndices) {

    @Override
    public boolean equals(Object other) {
        return other instanceof Building building
                && Arrays.equals(roomIndices, building.roomIndices);
    }

    @Override
    public int hashCode() {
        return Arrays.hashCode(roomIndices);
    }

    @Override
    public String toString() {
        return "Building[roomIndices=" + Arrays.toString(roomIndices) + "]";
    }
}
