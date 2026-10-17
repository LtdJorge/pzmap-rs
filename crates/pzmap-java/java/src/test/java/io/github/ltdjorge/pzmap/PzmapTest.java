package io.github.ltdjorge.pzmap;

import static org.junit.jupiter.api.Assertions.assertAll;
import static org.junit.jupiter.api.Assertions.assertArrayEquals;
import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertThrows;
import static org.junit.jupiter.api.Assertions.assertTrue;
import static org.junit.jupiter.api.Assumptions.assumeTrue;

import java.io.IOException;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;
import java.util.concurrent.Callable;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.concurrent.Future;
import java.util.concurrent.TimeUnit;
import org.junit.jupiter.api.Test;

/**
 * Tests for the pzmap Java bindings.
 *
 * <p>The tests that need real game data run only when {@code PZ_INSTALL_DIR} names a Project
 * Zomboid Build 42 install. Without it they are skipped, and the other tests still run. The game
 * tests check facts that hold for any Build 42 install, not counts from one version of the game.
 */
class PzmapTest {

    private static final String NO_INSTALL =
            "set PZ_INSTALL_DIR to a Project Zomboid Build 42 installation";
    private static final String INSTALL_DIR = System.getenv("PZ_INSTALL_DIR");
    private static final Path INSTALL =
            INSTALL_DIR == null || INSTALL_DIR.isEmpty() ? null : Path.of(INSTALL_DIR);
    private static final Path MAP_DIR =
            INSTALL == null ? null : INSTALL.resolve("media/maps/Muldraugh, KY");
    private static final Path PACK =
            INSTALL == null ? null : INSTALL.resolve("media/texturepacks/Tiles2x.pack");

    /** Cell 49,6 of vanilla Muldraugh, a town cell with rooms and buildings. */
    private static final int CELL_X = 49;
    private static final int CELL_Y = 6;

    /** Skips the calling test unless the Muldraugh map is installed. */
    private static void assumeGameInstalled() {
        assumeTrue(INSTALL != null, NO_INSTALL);
        assumeTrue(Files.isDirectory(MAP_DIR), "no Muldraugh map under " + INSTALL);
    }

    /** Skips the calling test unless {@code Tiles2x.pack} is installed. */
    private static void assumePackInstalled() {
        assumeTrue(INSTALL != null, NO_INSTALL);
        assumeTrue(Files.isRegularFile(PACK), "no Tiles2x.pack under " + INSTALL);
    }

    private static Path lotheaderPath() {
        return MAP_DIR.resolve(CELL_X + "_" + CELL_Y + ".lotheader");
    }

    private static Path lotpackPath() {
        return MAP_DIR.resolve("world_" + CELL_X + "_" + CELL_Y + ".lotpack");
    }

    /** A pack with one page, no textures and a four-byte image. */
    private static byte[] onePagePack() {
        ByteBuffer buffer = ByteBuffer.allocate(33).order(ByteOrder.LITTLE_ENDIAN);
        buffer.put(new byte[] {'P', 'Z', 'P', 'K'});
        buffer.putInt(1); // envelope version
        buffer.putInt(1); // page count
        buffer.putInt(1).put((byte) 'p'); // page name
        buffer.putInt(0); // texture count
        buffer.putInt(1); // has alpha
        buffer.putInt(4).put(new byte[] {(byte) 0x89, 'P', 'N', 'G'});
        return buffer.array();
    }

    @Test
    void buildOrdinalsMatchTheNativeBuilds() {
        assertAll(
                () -> assertEquals("Build 41", Build.B41.displayName()),
                () -> assertEquals("Build 42", Build.B42.displayName()));
    }

    @Test
    void malformedInputThrowsAStructuredException() {
        // "PZPK", envelope version 9, zero pages: a version this library does not support.
        byte[] bad = {'P', 'Z', 'P', 'K', 9, 0, 0, 0, 0, 0, 0, 0};

        PzmapParseException error =
                assertThrows(
                        PzmapParseException.class, () -> TexturePack.parse(bad));

        assertAll(
                () -> assertEquals("texture pack", error.getFormat()),
                () -> assertEquals(4, error.getOffset()),
                () -> assertTrue(error.getMessage().contains("9"), error.getMessage()));
    }

    @Test
    void aMissingFileIsAnIoException() {
        assertThrows(
                IOException.class, () -> TexturePack.parse(Path.of("/nonexistent/nope.pack")));
    }

    @Test
    void closingWhileOtherThreadsCallInIsSafe() throws Exception {
        byte[] data = onePagePack();
        int threads = 4;
        ExecutorService pool = Executors.newFixedThreadPool(threads);
        try {
            for (int round = 0; round < 50; round++) {
                TexturePack pack = TexturePack.parse(data);
                Page page = pack.pages().get(0);
                CountDownLatch started = new CountDownLatch(threads);

                // Each caller loops until the pack is closed under it. Any exception other than
                // IllegalStateException fails the test through Future.get.
                Callable<Void> caller =
                        () -> {
                            started.countDown();
                            while (true) {
                                try {
                                    assertEquals(4, page.png().length);
                                } catch (IllegalStateException closed) {
                                    return null;
                                }
                            }
                        };
                List<Future<Void>> results = new ArrayList<>();
                for (int i = 0; i < threads; i++) {
                    results.add(pool.submit(caller));
                }

                started.await();
                pack.close();
                for (Future<Void> result : results) {
                    result.get(10, TimeUnit.SECONDS);
                }
                assertTrue(pack.isClosed());
            }
        } finally {
            pool.shutdownNow();
        }
    }

    @Test
    void lotheader() throws IOException {
        assumeGameInstalled();
        try (LotHeader header = LotHeader.parse(lotheaderPath(), CELL_X, CELL_Y, Build.B42);
                LotHeader fromBytes =
                        LotHeader.parse(
                                Files.readAllBytes(lotheaderPath()), CELL_X, CELL_Y, Build.B42)) {
            List<Room> rooms = header.rooms();
            assertAll(
                    () -> assertEquals(CELL_X, header.x()),
                    () -> assertEquals(CELL_Y, header.y()),
                    () -> assertTrue(header.strings().length > 0),
                    () -> assertFalse(rooms.isEmpty()),
                    () -> assertFalse(header.buildings().isEmpty()),
                    () -> assertTrue(header.minLayer() >= Build.B42.minLayer()),
                    () -> assertTrue(header.minLayer() <= header.maxLayer()),
                    () -> assertTrue(header.maxLayer() <= Build.B42.maxLayer()),
                    () -> assertEquals(0, header.trailingBytes()));

            // Every building has to name rooms that exist.
            for (Building building : header.buildings()) {
                for (int room : building.roomIndices()) {
                    assertTrue(room >= 0 && room < rooms.size(), "room index " + room);
                }
            }

            // The zombie-population grid is one byte per block over a 32x32 grid for Build 42.
            assertEquals(32, header.zpopSide());
            assertEquals(32 * 32, header.zpop().length);
            assertEquals(header.zpop()[1 * 32 + 2] & 0xFF, header.zpopAt(1, 2));
            assertThrows(IndexOutOfBoundsException.class, () -> header.zpopAt(32, 0));

            Room room = rooms.get(0);
            assertFalse(room.name().isEmpty());
            assertFalse(room.rectangles().isEmpty());

            // Parsing from memory has to give the same header as parsing from disk.
            assertArrayEquals(header.strings(), fromBytes.strings());
            assertEquals(rooms, fromBytes.rooms());
            assertEquals(header.buildings(), fromBytes.buildings());
        }
    }

    @Test
    void cell() throws IOException {
        assumeGameInstalled();
        try (LotHeader header = LotHeader.parse(lotheaderPath(), CELL_X, CELL_Y, Build.B42);
                Cell cell =
                        Cell.parse(
                                lotpackPath(),
                                CELL_X,
                                CELL_Y,
                                Build.B42,
                                header.minLayer(),
                                header.maxLayer())) {

            int size = cell.size();
            int[] tiles = cell.tiles();
            long[] offsets = cell.offsets();
            int[] positions = cell.squareData();

            assertAll(
                    () -> assertTrue(size > 0),
                    () -> assertEquals(cell.tileCount(), tiles.length),
                    () -> assertEquals(size + 1, offsets.length),
                    () -> assertEquals(size * Cell.SQUARE_STRIDE, positions.length),
                    () -> assertEquals(0, offsets[0]),
                    () -> assertEquals(tiles.length, offsets[offsets.length - 1]));

            // The view over the bulk arrays has to agree with the per-square native call, including
            // for the last square, which is where an off-by-one in the offsets would show up.
            List<Square> squares = cell.squares();
            assertEquals(size, squares.size());

            for (int index : new int[] {0, 1, size / 2, size - 1}) {
                Square square = squares.get(index);
                assertArrayEquals(cell.tilesOf(index), square.tiles(), "square " + index);
                assertEquals(positions[index * Cell.SQUARE_STRIDE], square.block());
                assertEquals(positions[index * Cell.SQUARE_STRIDE + 1], square.layer());
            }

            // Every tile index has to address the header's string table, or the two files disagree.
            int names = header.strings().length;
            for (int tile : tiles) {
                assertTrue(tile >= 0 && tile < names, "tile index " + tile);
            }

            assertThrows(IndexOutOfBoundsException.class, () -> squares.get(size));
            assertThrows(IndexOutOfBoundsException.class, () -> cell.tilesOf(size));
            assertThrows(IndexOutOfBoundsException.class, () -> cell.tilesOf(-1));
        }
    }

    @Test
    void texturePack() throws IOException {
        assumePackInstalled();
        try (TexturePack pack = TexturePack.parse(PACK)) {
            List<Page> pages = pack.pages();
            assertFalse(pages.isEmpty());
            assertEquals(pack.pageCount(), pages.size());
            assertEquals(
                    pack.textureCount(),
                    pages.stream().mapToInt(page -> page.textures().size()).sum());

            Page page = pages.get(0);
            assertFalse(page.name().isEmpty());
            assertFalse(page.textures().isEmpty());

            // Pages are handed over PNG-compressed and undecoded, so the signature is still there.
            byte[] png = page.png();
            assertTrue(png.length > 8);
            assertEquals((byte) 0x89, png[0]);
            assertEquals('P', png[1]);
        }
    }

    @Test
    void closingIsIdempotentAndUseAfterCloseThrows() throws IOException {
        TexturePack pack = TexturePack.parse(onePagePack());
        Page page = pack.pages().get(0);
        assertFalse(pack.isClosed());

        pack.close();
        assertTrue(pack.isClosed());

        // A second close must not double-free.
        pack.close();

        assertThrows(IllegalStateException.class, page::png);
    }
}
