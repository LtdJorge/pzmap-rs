package io.github.ltdjorge.pzmap;

import java.io.IOException;
import java.nio.file.Path;
import java.util.AbstractList;
import java.util.Arrays;
import java.util.List;

/**
 * The non-empty squares of a parsed {@code world_X_Y.lotpack} file, and the tiles on each.
 *
 * <p>A square's tiles are stored as indices into the tile names of the matching
 * {@link LotHeader}. The lotpack does not record its own layer range either, so it is parsed with
 * the range from the header.
 *
 * <p>A cell holds hundreds of thousands of squares and millions of tile indices. Calling into
 * native code once per square would be slow, so {@link #tiles()}, {@link #offsets()} and
 * {@link #squareData()} each return the data for the whole cell in one call. {@link #squares()}
 * wraps those three arrays in a list of {@link Square} records.
 *
 * <p>The parsed data is held in native memory, outside the Java heap. Close the cell when you are
 * done with it, ideally with try-with-resources:
 *
 * <pre>{@code
 * try (LotHeader header = LotHeader.parse(headerPath, 49, 6, Build.B42);
 *      Cell cell = Cell.parse(packPath, 49, 6, Build.B42, header.minLayer(), header.maxLayer())) {
 *     String[] names = header.strings();
 *     for (Square square : cell.squares()) {
 *         for (int tile : square.tiles()) {
 *             System.out.println(names[tile]);
 *         }
 *     }
 * }
 * }</pre>
 *
 * <p>After {@link #close()}, the methods that read native memory throw
 * {@link IllegalStateException}. {@link #x()}, {@link #y()}, {@link #size()} and
 * {@link #tileCount()} keep working, and so do arrays and lists returned earlier. See the
 * {@linkplain io.github.ltdjorge.pzmap package documentation} for the rest of the rules on closing
 * and threads.
 */
public final class Cell implements AutoCloseable {

    /** The number of ints per square in {@link #squareData()}: block, layer, x, y and room. */
    public static final int SQUARE_STRIDE = 5;

    static {
        NativeLoader.ensureLoaded();
    }

    private final NativeHandle handle;
    private final int x;
    private final int y;
    private final int squareCount;
    private final int tileCount;

    private Cell(long pointer) {
        this.handle = new NativeHandle(pointer, "Cell", Cell::nativeFree);

        int[] summary = handle.call(Cell::nativeSummary);
        this.x = summary[0];
        this.y = summary[1];
        this.squareCount = summary[2];
        this.tileCount = summary[3];
    }

    /**
     * Reads and parses a {@code world_X_Y.lotpack} file.
     *
     * @param path the {@code .lotpack} file
     * @param x the cell's X coordinate, from the file name
     * @param y the cell's Y coordinate, from the file name
     * @param build the game build that wrote the file
     * @param minLayer the lowest layer, from {@link LotHeader#minLayer()} of the matching header
     * @param maxLayer the end of the layer range, from {@link LotHeader#maxLayer()} of the
     *     matching header
     * @return the parsed cell, which the caller must close
     * @throws PzmapParseException if the file is malformed or was written by another build
     * @throws IOException if the file cannot be read
     */
    public static Cell parse(Path path, int x, int y, Build build, int minLayer, int maxLayer)
            throws IOException {
        return new Cell(
                nativeParsePath(path.toString(), x, y, build.ordinal(), minLayer, maxLayer));
    }

    /**
     * Parses a {@code world_X_Y.lotpack} from a byte array.
     *
     * <p>The native library copies {@code data}, so the array can be reused afterwards.
     *
     * @param data the contents of a {@code .lotpack} file
     * @param x the cell's X coordinate, from the file name
     * @param y the cell's Y coordinate, from the file name
     * @param build the game build that wrote the data
     * @param minLayer the lowest layer, from {@link LotHeader#minLayer()} of the matching header
     * @param maxLayer the end of the layer range, from {@link LotHeader#maxLayer()} of the
     *     matching header
     * @return the parsed cell, which the caller must close
     * @throws PzmapParseException if the data is malformed or was written by another build
     */
    public static Cell parse(byte[] data, int x, int y, Build build, int minLayer, int maxLayer)
            throws PzmapParseException {
        return new Cell(nativeParseBytes(data, x, y, build.ordinal(), minLayer, maxLayer));
    }

    /** {@return the cell's X coordinate, as passed to {@code parse}} */
    public int x() {
        return x;
    }

    /** {@return the cell's Y coordinate, as passed to {@code parse}} */
    public int y() {
        return y;
    }

    /** {@return the number of non-empty squares in the cell} */
    public int size() {
        return squareCount;
    }

    /** {@return the total number of tile indices across all squares} */
    public int tileCount() {
        return tileCount;
    }

    /**
     * {@return the tile indices of every square, concatenated into one array}
     *
     * <p>The tiles of square {@code i} run from {@code offsets()[i]} up to, but not including,
     * {@code offsets()[i + 1]}. Within a square, tiles are in draw order, bottom first. Each value
     * is an index into {@link LotHeader#strings()}.
     *
     * <p>Each call builds a new array, so keep the result instead of calling this in a loop.
     *
     * @throws IllegalStateException if this cell has been closed
     */
    public int[] tiles() {
        return handle.call(Cell::nativeTiles);
    }

    /**
     * {@return where each square's tiles start in {@link #tiles()}}
     *
     * <p>The array has {@link #size()} + 1 entries. The last one is the total tile count, so the
     * tiles of the last square end there like any other.
     *
     * @throws IllegalStateException if this cell has been closed
     */
    public long[] offsets() {
        return handle.call(Cell::nativeOffsets);
    }

    /**
     * {@return the position of every square, {@link #SQUARE_STRIDE} ints per square}
     *
     * <p>Square {@code i} starts at index {@code i * SQUARE_STRIDE}. Its five values mean the same
     * as the fields of {@link Square}: block, layer, x, y and room.
     *
     * @throws IllegalStateException if this cell has been closed
     */
    public int[] squareData() {
        return handle.call(Cell::nativeSquares);
    }

    /**
     * Returns the tile indices of one square.
     *
     * <p>This makes one native call per square. To read many squares, use {@link #squares()} or
     * {@link #tiles()} instead.
     *
     * @param square the square's index, from 0 to {@code size() - 1}
     * @return the square's tile indices, in draw order
     * @throws IndexOutOfBoundsException if {@code square} is out of range
     * @throws IllegalStateException if this cell has been closed
     */
    public int[] tilesOf(int square) {
        return handle.call(pointer -> nativeTilesOf(pointer, square));
    }

    /**
     * Returns every non-empty square, in file order.
     *
     * <p>This method fetches {@link #tiles()}, {@link #offsets()} and {@link #squareData()} once.
     * The list creates each {@link Square} when you read it, without calling native code, so it
     * keeps working after the cell is closed.
     *
     * @return an unmodifiable list of {@link #size()} squares
     * @throws IllegalStateException if this cell has been closed
     */
    public List<Square> squares() {
        int[] positions = squareData();
        int[] tiles = tiles();
        long[] offsets = offsets();

        return new AbstractList<>() {
            @Override
            public Square get(int index) {
                if (index < 0 || index >= squareCount) {
                    throw new IndexOutOfBoundsException(
                            "square " + index + " of " + squareCount);
                }
                int base = index * SQUARE_STRIDE;
                int start = Math.toIntExact(offsets[index]);
                int end = Math.toIntExact(offsets[index + 1]);
                return new Square(
                        positions[base],
                        positions[base + 1],
                        positions[base + 2],
                        positions[base + 3],
                        positions[base + 4],
                        Arrays.copyOfRange(tiles, start, end));
            }

            @Override
            public int size() {
                return squareCount;
            }
        };
    }

    /** {@return whether {@link #close()} has been called} */
    public boolean isClosed() {
        return handle.isClosed();
    }

    /**
     * Frees the native memory behind this cell.
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
        return "Cell[x=" + x + ", y=" + y + ", squares=" + squareCount + ", tiles=" + tileCount + "]";
    }

    private static native long nativeParsePath(
            String path, int x, int y, int build, int minLayer, int maxLayer);

    private static native long nativeParseBytes(
            byte[] data, int x, int y, int build, int minLayer, int maxLayer);

    private static native void nativeFree(long handle);

    private static native int[] nativeSummary(long handle);

    private static native int[] nativeTiles(long handle);

    private static native long[] nativeOffsets(long handle);

    private static native int[] nativeSquares(long handle);

    private static native int[] nativeTilesOf(long handle, int square);
}
