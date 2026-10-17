# pzmap for Java

Java bindings for [`libpzmap`](https://github.com/LtdJorge/pzmap-rs), a Rust library that parses
Project Zomboid's map files: `.lotheader`, `world_*.lotpack`, and `.pack` texture atlases.

The parsing runs in a native library, which the jar contains and loads for you. The bindings
return the file contents as plain Java values. They do not decode images or draw anything. A
texture page comes back as PNG bytes, and a square's tiles come back as indices into its cell's
list of tile names.

## Layout

- `src/` holds the native side, a Cargo crate that builds `libpzmap_jni.so`.
- `java/src/main/java` holds the Java API, in package `io.github.ltdjorge.pzmap`.
- `java/src/test/java` holds the JUnit 5 tests.
- `build.gradle.kts` builds both and puts the native library into the jar.

## Build

```sh
./gradlew build          # compiles the JNI library and the Java sources, then jars them together
./gradlew test           # runs the JUnit suite against the freshly built library
./gradlew publishToMavenLocal
```

The Gradle build runs `cargo build -p pzmap-java` and copies the resulting library into the jar
under `native/<os>-<arch>/`. At runtime, `NativeLoader` extracts it from there and loads it.

The wrapper downloads the Gradle version the build needs, so you don't need to install Gradle. On
Windows, use `gradlew.bat`.

## Use

Each cell of the map is stored as two files, and you need both. The `.lotheader` lists every tile
name the cell uses. The matching `world_X_Y.lotpack` places tiles on squares by their index into
that list.

```java
import io.github.ltdjorge.pzmap.*;
import java.nio.file.Path;

Path dir = Path.of("/opt/ProjectZomboid/media/maps/Muldraugh, KY");

try (LotHeader header = LotHeader.parse(dir.resolve("49_6.lotheader"), 49, 6, Build.B42);
     Cell cell = Cell.parse(dir.resolve("world_49_6.lotpack"), 49, 6, Build.B42,
                            header.minLayer(), header.maxLayer())) {

    String[] names = header.strings();
    for (Square square : cell.squares()) {
        for (int tile : square.tiles()) {
            System.out.println(square.layer() + " " + names[tile]);
        }
    }
}
```

`x` and `y` must be the cell coordinates in the file name. The parser uses them to convert room and
object positions to world coordinates. Wrong values do not cause an error, but every room ends up in
the wrong place.

## Memory

`LotHeader`, `Cell` and `TexturePack` keep their parsed data in native memory, outside the Java
heap. They implement `AutoCloseable`, so open them with try-with-resources. If you forget to close
one, the garbage collector frees its memory eventually. That can take a long time, and a texture
pack from the base game holds hundreds of megabytes.

After `close()`:

- Methods that read native memory throw `IllegalStateException`.
- Values read during parsing, such as `x()`, `size()` or `pages()`, keep working.
- Arrays and lists returned earlier keep working, because they are copies.
- `Page.png()` throws `IllegalStateException`, because the image is still in native memory.
- Calling `close()` again does nothing.

All three classes can be shared between threads. Calls from several threads run at the same time,
and `close()` waits for calls in progress to finish before it frees the memory.

## Bulk access

A cell holds hundreds of thousands of squares and millions of tile indices. Calling into native code
once per square would be slow, so the API hands over whole cells at once.

`Cell.tiles()`, `Cell.offsets()` and `Cell.squareData()` each return the data for the whole cell in
one call. `Cell.squares()` wraps those three arrays in a list, so iterating it never calls native
code. Each of these methods builds new arrays on every call, so keep the result instead of calling
it in a loop.

```java
int[] tiles = cell.tiles();
long[] offsets = cell.offsets();

// The tiles of square i run from tiles[offsets[i]] up to, but not including, tiles[offsets[i + 1]].
```

`offsets` has one more entry than there are squares, so the last square needs no special case.

## Builds

Build 41 and Build 42 use different cell and block sizes and different layer ranges. A file read as
the wrong build often parses without an error but gives wrong values. So every parse method takes a
`Build` argument instead of guessing the build from the file.

## Errors

Malformed input throws `PzmapParseException`, which extends `IOException`. `getOffset()` returns
the byte offset where parsing failed, and `getFormat()` returns which format was being read.

An unreadable file throws a plain `IOException`.

A `RuntimeException` whose message starts with `libpzmap panicked` means a bug in this library,
not a problem with your file. Please report it.

## Native access

The native library is loaded the first time you use any class in the package. The loader looks in
three places, in this order:

1. The file named by the `pzmap.native.library` system property.
2. The copy inside the jar under `native/<os>-<arch>/`. It is copied to a temporary file first,
   which is deleted when the JVM exits.
3. The directories in `java.library.path`.

If none of them works, that first use throws `UnsatisfiedLinkError`.

From JDK 24, loading a native library is a restricted operation. Run with
`--enable-native-access=ALL-UNNAMED`, or the JVM prints a warning on first use. The Gradle test task
already passes it.

To test against a library cargo just built, point the property at it:

```sh
java -Dpzmap.native.library=target/release/libpzmap_jni.so ...
```

## Requirements

Java 17 or newer.

## License

MIT OR Apache-2.0.

The jar's `META-INF/THIRD-PARTY-NOTICES.md` lists the licenses of the Rust crates compiled into its
native library.

Project Zomboid is a trademark of The Indie Stone. This project is not affiliated with or endorsed
by The Indie Stone, and it does not include any game files.
