/**
 * Reads Project Zomboid map files: lot headers, lotpacks (cells) and texture packs.
 *
 * <p>Each cell of the map is stored as two files. A {@code .lotheader}, read with
 * {@link io.github.ltdjorge.pzmap.LotHeader}, lists the tile names the cell uses, plus its rooms,
 * buildings and zombie population. The matching {@code world_X_Y.lotpack}, read with
 * {@link io.github.ltdjorge.pzmap.Cell}, says which tiles stand on each square, as indexes into the
 * header's tile names. Texture packs, read with {@link io.github.ltdjorge.pzmap.TexturePack}, hold
 * the sprite images those names refer to.
 *
 * <h2>Closing</h2>
 *
 * <p>{@code LotHeader}, {@code Cell} and {@code TexturePack} keep their parsed data in native
 * memory, outside the Java heap. Close them when you are done, ideally with try-with-resources.
 * After {@code close()}, methods that read native memory throw
 * {@link java.lang.IllegalStateException}. Each class lists the methods that keep working. Closing
 * twice does nothing. If you never close an object, the garbage collector frees its memory
 * eventually, but large cells and texture packs can hold hundreds of megabytes until then.
 *
 * <p>Arrays and lists these classes return are copies, so they stay valid after {@code close()}.
 *
 * <h2>Threads</h2>
 *
 * <p>All three classes can be used from several threads at once. Calls run in parallel, and
 * {@code close()} waits for calls in progress before it frees the memory.
 *
 * <h2>Errors</h2>
 *
 * <p>A malformed file throws {@link io.github.ltdjorge.pzmap.PzmapParseException}, which records
 * the byte offset and the file format. A {@code RuntimeException} whose message starts with
 * "libpzmap panicked" means a bug in this library. Please report it.
 */
package io.github.ltdjorge.pzmap;
