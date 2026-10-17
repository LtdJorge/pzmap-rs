package io.github.ltdjorge.pzmap;

/**
 * The game build a map file was written by.
 *
 * <p>Build 41 and Build 42 use different cell and block sizes and different layer ranges. A layer
 * is one level of the map. Level 0 is the ground and negative levels are basements.
 *
 * <p>A file read as the wrong build often parses without an error but gives wrong values. So every
 * parse method takes the build as an argument instead of guessing it from the file.
 *
 * <p>The native library receives each constant as its {@link #ordinal()}, so the order of the
 * constants must not change.
 */
public enum Build {
    /** Build 41: a cell is 30x30 blocks of 10x10 squares, with layers 0 to 8. */
    B41,
    /** Build 42: a cell is 32x32 blocks of 8x8 squares, with layers -32 to 32. */
    B42;

    static {
        NativeLoader.ensureLoaded();
    }

    private int[] geometry;

    /**
     * Returns the number of blocks along one edge of a cell.
     *
     * @return 30 for Build 41, 32 for Build 42
     */
    public int cellSizeInBlocks() {
        return geometry()[0];
    }

    /**
     * Returns the number of squares along one edge of a block.
     *
     * @return 10 for Build 41, 8 for Build 42
     */
    public int blockSizeInSquares() {
        return geometry()[1];
    }

    /**
     * Returns the number of squares along one edge of a cell.
     *
     * @return {@link #cellSizeInBlocks()} times {@link #blockSizeInSquares()}: 300 for Build 41,
     *     256 for Build 42
     */
    public int cellSizeInSquares() {
        return geometry()[2];
    }

    /** {@return the lowest level this build can store} */
    public int minLayer() {
        return geometry()[3];
    }

    /**
     * {@return the upper end of this build's layer range}
     *
     * <p>{@link LotHeader#maxLayer()} is never larger than this value.
     */
    public int maxLayer() {
        return geometry()[4];
    }

    /**
     * Returns the version number this build writes into {@code .lotheader} and {@code .lotpack}
     * files.
     *
     * @return the version field value
     */
    public int versionNumber() {
        return geometry()[5];
    }

    /**
     * Returns the build's display name.
     *
     * @return a name such as {@code "Build 42"}
     */
    public String displayName() {
        return nativeName(ordinal());
    }

    /** Fetches the build's constants from native code on first use and caches them. */
    private int[] geometry() {
        int[] cached = geometry;
        if (cached == null) {
            cached = nativeGeometry(ordinal());
            geometry = cached;
        }
        return cached;
    }

    private static native int[] nativeGeometry(int build);

    private static native String nativeName(int build);
}
