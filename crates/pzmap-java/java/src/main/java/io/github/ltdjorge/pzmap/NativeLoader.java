package io.github.ltdjorge.pzmap;

import java.io.IOException;
import java.io.InputStream;
import java.io.UncheckedIOException;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.StandardCopyOption;
import java.util.Locale;

/**
 * Finds and loads the native library that does the parsing.
 *
 * <p>The library is found in one of three places, tried in this order:
 *
 * <ol>
 *   <li>The file named by the {@code pzmap.native.library} system property. Development builds use
 *       this to point at the file cargo just built.
 *   <li>A copy inside the jar under {@code /native/<os>-<arch>/}. It is copied to a temporary file,
 *       loaded from there, and deleted when the JVM exits. Released jars use this.
 *   <li>{@link System#loadLibrary}, which searches {@code java.library.path}.
 * </ol>
 *
 * <p>Loading happens once, the first time any public class in this package is used. If the system
 * property is set, the other two places are not tried. If loading fails, that first use throws
 * {@link UnsatisfiedLinkError}, and every later use of the class throws
 * {@link NoClassDefFoundError}. If the copy in the jar cannot be written to a temporary file, the
 * first use throws {@link ExceptionInInitializerError} instead.
 */
final class NativeLoader {

    /** The system property that names the library file directly. */
    static final String LIBRARY_PROPERTY = "pzmap.native.library";

    private static final String LIBRARY_NAME = "pzmap_jni";

    private NativeLoader() {}

    static {
        load();
    }

    /**
     * Loads the native library if it is not loaded yet.
     *
     * <p>Every public class calls this from its static initializer, so users of the package never
     * need to.
     */
    static void ensureLoaded() {
        // Calling any static method runs the static block above first, which does the loading.
    }

    // From JDK 24, loading a native library is a restricted operation. Applications allow it with
    // --enable-native-access.
    @SuppressWarnings("restricted")
    private static void load() {
        String explicit = System.getProperty(LIBRARY_PROPERTY);
        if (explicit != null && !explicit.isEmpty()) {
            System.load(Path.of(explicit).toAbsolutePath().toString());
            return;
        }

        String resource = "/native/" + platform() + "/" + System.mapLibraryName(LIBRARY_NAME);
        try (InputStream stream = NativeLoader.class.getResourceAsStream(resource)) {
            if (stream != null) {
                System.load(extract(stream).toString());
                return;
            }
        } catch (IOException e) {
            throw new UncheckedIOException("could not extract " + resource, e);
        }

        // The jar has no library for this platform, so search java.library.path. If that fails
        // too, the error message names the system property that would fix it.
        try {
            System.loadLibrary(LIBRARY_NAME);
        } catch (UnsatisfiedLinkError e) {
            throw new UnsatisfiedLinkError(
                    "no native library for this platform in the jar ("
                            + resource
                            + "), none on the library path, and -D"
                            + LIBRARY_PROPERTY
                            + " is not set: "
                            + e.getMessage());
        }
    }

    /** Copies the bundled library to a temporary file, because {@code System.load} needs a file. */
    private static Path extract(InputStream stream) throws IOException {
        Path file = Files.createTempFile(LIBRARY_NAME, suffix());
        file.toFile().deleteOnExit();
        Files.copy(stream, file, StandardCopyOption.REPLACE_EXISTING);
        return file.toAbsolutePath();
    }

    private static String suffix() {
        String mapped = System.mapLibraryName(LIBRARY_NAME);
        int dot = mapped.lastIndexOf('.');
        return dot < 0 ? "" : mapped.substring(dot);
    }

    /** Returns the {@code <os>-<arch>} directory name that the Gradle build puts the library in. */
    private static String platform() {
        String os = System.getProperty("os.name", "").toLowerCase(Locale.ROOT);
        String arch = System.getProperty("os.arch", "").toLowerCase(Locale.ROOT);

        String osName;
        if (os.contains("win")) {
            osName = "windows";
        } else if (os.contains("mac") || os.contains("darwin")) {
            osName = "macos";
        } else {
            osName = "linux";
        }

        String archName;
        if (arch.equals("amd64") || arch.equals("x86_64")) {
            archName = "x86_64";
        } else if (arch.equals("aarch64") || arch.equals("arm64")) {
            archName = "aarch64";
        } else {
            archName = arch;
        }

        return osName + "-" + archName;
    }
}
