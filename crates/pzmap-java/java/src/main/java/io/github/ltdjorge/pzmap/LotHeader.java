package io.github.ltdjorge.pzmap;

import java.io.IOException;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.List;

/**
 * A parsed {@code .lotheader} file: the tile names, rooms, buildings and zombie population of one
 * cell.
 *
 * <p>Each cell is stored as two files. The {@code .lotheader} lists every tile name the cell uses.
 * The matching {@code world_X_Y.lotpack}, read with {@link Cell}, places tiles on squares by their
 * index into {@link #strings()}. You need both files to know which tile is where.
 *
 * <p>The parsed data is held in native memory, outside the Java heap. Close the header when you
 * are done with it, ideally with try-with-resources:
 *
 * <pre>{@code
 * try (LotHeader header = LotHeader.parse(Path.of("0_18.lotheader"), 0, 18, Build.B42)) {
 *     System.out.println(header.strings().length + " distinct tile names");
 * }
 * }</pre>
 *
 * <p>After {@link #close()}, the methods that read native memory throw
 * {@link IllegalStateException}. The values read during parsing keep working: {@link #x()},
 * {@link #y()}, {@link #version()}, the layer range and {@link #trailingBytes()}. Arrays and lists
 * returned earlier are copies, so they keep working too. See the
 * {@linkplain io.github.ltdjorge.pzmap package documentation} for the rest of the rules on closing
 * and threads.
 */
public final class LotHeader implements AutoCloseable {

    static {
        NativeLoader.ensureLoaded();
    }

    private final NativeHandle handle;
    private final int x;
    private final int y;
    private final int version;
    private final int minLayer;
    private final int maxLayer;
    private final int trailingBytes;

    private LotHeader(long pointer) {
        this.handle = new NativeHandle(pointer, "LotHeader", LotHeader::nativeFree);

        // Read all six values in one native call and keep them on the Java side.
        int[] summary = handle.call(LotHeader::nativeSummary);
        this.x = summary[0];
        this.y = summary[1];
        this.version = summary[2];
        this.minLayer = summary[3];
        this.maxLayer = summary[4];
        this.trailingBytes = summary[5];
    }

    /**
     * Reads and parses a {@code .lotheader} file.
     *
     * <p>{@code x} and {@code y} must be the cell coordinates in the file name, such as 49 and 6
     * for {@code 49_6.lotheader}. The parser uses them to convert room and object positions to
     * world coordinates. Wrong values do not cause an error, but every room ends up in the wrong
     * place.
     *
     * @param path the {@code .lotheader} file
     * @param x the cell's X coordinate, from the file name
     * @param y the cell's Y coordinate, from the file name
     * @param build the game build that wrote the file
     * @return the parsed header, which the caller must close
     * @throws PzmapParseException if the file is malformed or was written by another build
     * @throws IOException if the file cannot be read
     */
    public static LotHeader parse(Path path, int x, int y, Build build) throws IOException {
        return new LotHeader(nativeParsePath(path.toString(), x, y, build.ordinal()));
    }

    /**
     * Parses a {@code .lotheader} from a byte array.
     *
     * <p>The native library copies {@code data}, so the array can be reused afterwards.
     *
     * @param data the contents of a {@code .lotheader} file
     * @param x the cell's X coordinate, from the file name
     * @param y the cell's Y coordinate, from the file name
     * @param build the game build that wrote the data
     * @return the parsed header, which the caller must close
     * @throws PzmapParseException if the data is malformed or was written by another build
     * @see #parse(Path, int, int, Build)
     */
    public static LotHeader parse(byte[] data, int x, int y, Build build)
            throws PzmapParseException {
        return new LotHeader(nativeParseBytes(data, x, y, build.ordinal()));
    }

    /** {@return the cell's X coordinate, as passed to {@code parse}} */
    public int x() {
        return x;
    }

    /** {@return the cell's Y coordinate, as passed to {@code parse}} */
    public int y() {
        return y;
    }

    /** {@return the version number stored in the file} See {@link Build#versionNumber()}. */
    public int version() {
        return version;
    }

    /**
     * Returns the lowest layer this cell stores.
     *
     * <p>Pass this to {@link Cell#parse(Path, int, int, Build, int, int)} along with
     * {@link #maxLayer()}.
     *
     * @return the lowest layer, inclusive
     */
    public int minLayer() {
        return minLayer;
    }

    /**
     * Returns one more than the highest layer this cell stores.
     *
     * @return the end of the layer range, exclusive
     */
    public int maxLayer() {
        return maxLayer;
    }

    /**
     * {@return the number of bytes left over after the last field the parser knows about}
     *
     * <p>This is normally 0. Any other value suggests the file uses a layout this library does not
     * know, and the parsed values may be wrong.
     */
    public int trailingBytes() {
        return trailingBytes;
    }

    /**
     * {@return every distinct tile name the cell uses}
     *
     * <p>The tile indices in a {@link Cell} point into this array. Each call builds a new array,
     * so keep the result instead of calling this in a loop.
     *
     * @throws IllegalStateException if this header has been closed
     */
    public String[] strings() {
        return handle.call(LotHeader::nativeStrings);
    }

    /**
     * {@return the base zombie population of every block in the cell}
     *
     * <p>The grid has {@link #zpopSide()} blocks along each edge. The value for a block is at index
     * {@code blockX * zpopSide() + blockY}. Read each byte as unsigned, or use {@link #zpopAt}.
     *
     * @throws IllegalStateException if this header has been closed
     */
    public byte[] zpop() {
        return handle.call(LotHeader::nativeZpop);
    }

    /**
     * Returns the number of blocks along one edge of the zombie population grid.
     *
     * @return the same value as {@link Build#cellSizeInBlocks()}
     * @throws IllegalStateException if this header has been closed
     */
    public int zpopSide() {
        return handle.call(LotHeader::nativeZpopSide);
    }

    /**
     * Returns the base zombie population of one block.
     *
     * @param blockX the block's X position within the cell, from 0 to {@code zpopSide() - 1}
     * @param blockY the block's Y position within the cell, from 0 to {@code zpopSide() - 1}
     * @return the population, from 0 to 255
     * @throws IndexOutOfBoundsException if either coordinate is outside the grid
     * @throws IllegalStateException if this header has been closed
     */
    public int zpopAt(int blockX, int blockY) {
        int value = handle.call(pointer -> nativeZpopAt(pointer, blockX, blockY));
        if (value < 0) {
            int side = zpopSide();
            throw new IndexOutOfBoundsException(
                    "block " + blockX + "," + blockY + " is outside a " + side + "x" + side + " grid");
        }
        return value;
    }

    /**
     * Returns the rooms in this cell, with positions in world coordinates.
     *
     * <p>Each call builds a new list.
     *
     * @return an unmodifiable list of rooms, in file order
     * @throws IllegalStateException if this header has been closed
     */
    public List<Room> rooms() {
        Object[] flat = handle.call(LotHeader::nativeRooms);
        String[] names = (String[]) flat[0];
        int[] layers = (int[]) flat[1];
        int[] rectangleStarts = (int[]) flat[2];
        int[] rectangles = (int[]) flat[3];
        int[] objectStarts = (int[]) flat[4];
        int[] objects = (int[]) flat[5];

        List<Room> rooms = new ArrayList<>(names.length);
        for (int index = 0; index < names.length; index++) {
            List<Rectangle> roomRectangles = new ArrayList<>();
            for (int i = rectangleStarts[index]; i < rectangleStarts[index + 1]; i += 4) {
                roomRectangles.add(
                        new Rectangle(
                                rectangles[i], rectangles[i + 1], rectangles[i + 2], rectangles[i + 3]));
            }
            List<RoomObject> roomObjects = new ArrayList<>();
            for (int i = objectStarts[index]; i < objectStarts[index + 1]; i += 3) {
                roomObjects.add(new RoomObject(objects[i], objects[i + 1], objects[i + 2]));
            }
            rooms.add(
                    new Room(
                            names[index],
                            layers[index],
                            List.copyOf(roomRectangles),
                            List.copyOf(roomObjects)));
        }
        return List.copyOf(rooms);
    }

    /**
     * Returns the buildings in this cell.
     *
     * @return an unmodifiable list of buildings, each naming its rooms by index into
     *     {@link #rooms()}
     * @throws IllegalStateException if this header has been closed
     */
    public List<Building> buildings() {
        Object[] flat = handle.call(LotHeader::nativeBuildings);
        int[] starts = (int[]) flat[0];
        int[] rooms = (int[]) flat[1];

        List<Building> buildings = new ArrayList<>(starts.length - 1);
        for (int index = 0; index + 1 < starts.length; index++) {
            buildings.add(new Building(Arrays.copyOfRange(rooms, starts[index], starts[index + 1])));
        }
        return List.copyOf(buildings);
    }

    /** {@return whether {@link #close()} has been called} */
    public boolean isClosed() {
        return handle.isClosed();
    }

    /**
     * Frees the native memory behind this header.
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
        return "LotHeader[x=" + x + ", y=" + y + ", layers=" + minLayer + ".." + maxLayer + "]";
    }

    private static native long nativeParsePath(String path, int x, int y, int build);

    private static native long nativeParseBytes(byte[] data, int x, int y, int build);

    private static native void nativeFree(long handle);

    private static native int[] nativeSummary(long handle);

    private static native String[] nativeStrings(long handle);

    private static native byte[] nativeZpop(long handle);

    private static native int nativeZpopSide(long handle);

    private static native int nativeZpopAt(long handle, int blockX, int blockY);

    private static native Object[] nativeRooms(long handle);

    private static native Object[] nativeBuildings(long handle);
}
