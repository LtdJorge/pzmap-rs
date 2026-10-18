import com.vanniktech.maven.publish.JavaLibrary
import com.vanniktech.maven.publish.JavadocJar
import com.vanniktech.maven.publish.SourcesJar
import org.gradle.internal.os.OperatingSystem

plugins {
    `java-library`
    id("com.vanniktech.maven.publish") version "0.37.0"
}

// The version comes from this crate's Cargo.toml, so a release changes it in one place. The
// first `version` line there is the package's.
version = Regex("""(?m)^version = "([^"]+)"""")
    .find(file("Cargo.toml").readText())
    ?.groupValues
    ?.get(1)
    ?: error("Cargo.toml has no version line")

java {
    // The API uses records, and 17 is the oldest long-term-support Java release that has them.
    toolchain { languageVersion = JavaLanguageVersion.of(17) }
}

// The Java sources live under `java/`, so the crate root stays a normal Cargo package.
sourceSets {
    named("main") { java.srcDir("java/src/main/java") }
    named("test") { java.srcDir("java/src/test/java") }
}

repositories { mavenCentral() }

dependencies {
    testImplementation(platform("org.junit:junit-bom:5.11.4"))
    testImplementation("org.junit.jupiter:junit-jupiter")
    testRuntimeOnly("org.junit.platform:junit-platform-launcher")
}

/** The cargo profile to build with: `release` by default, or `-PcargoProfile=debug`. */
val cargoProfile = (findProperty("cargoProfile") as String?) ?: "release"
val cargoTargetDir = layout.projectDirectory.dir("../../target/$cargoProfile")

/**
 * A directory of native libraries built beforehand, laid out as `native/<os>-<arch>/<file>`, from
 * `-PprebuiltNatives=<dir>`.
 *
 * The release workflow builds the library on each platform and passes them all here, so one jar
 * holds them all. Without it, the jar holds only this machine's library, built with cargo.
 */
val prebuiltNatives = findProperty("prebuiltNatives") as String?

/**
 * A native library file for the tests to load instead of cargo's, from `-PnativeLibrary=<file>`.
 *
 * The release workflow uses it to test the exact file it ships. With it, nothing runs cargo, and
 * a jar built without `-PprebuiltNatives` holds no native library.
 */
val testNativeLibrary = findProperty("nativeLibrary") as String?

/** The `<os>-<arch>` name of the jar directory that holds the native library. */
val nativePlatform: String by lazy {
    val os = OperatingSystem.current()
    val osName = when {
        os.isWindows -> "windows"
        os.isMacOsX -> "macos"
        else -> "linux"
    }
    val archName = when (val arch = System.getProperty("os.arch").lowercase()) {
        "amd64", "x86_64" -> "x86_64"
        "aarch64", "arm64" -> "aarch64"
        else -> arch
    }
    "$osName-$archName"
}

val nativeLibraryName: String by lazy {
    val os = OperatingSystem.current()
    when {
        os.isWindows -> "pzmap_jni.dll"
        os.isMacOsX -> "libpzmap_jni.dylib"
        else -> "libpzmap_jni.so"
    }
}

/**
 * Builds the native library with cargo.
 *
 * Gradle runs this every time and leaves it to cargo to decide whether anything needs rebuilding.
 * When nothing changed, cargo finishes almost at once.
 */
val cargoBuild by tasks.registering(Exec::class) {
    group = "build"
    description = "Compiles the pzmap-java JNI library with cargo."
    workingDir = layout.projectDirectory.asFile
    commandLine = buildList {
        add("cargo")
        add("build")
        add("-p")
        add("pzmap-java")
        if (cargoProfile == "release") add("--release")
    }
    outputs.upToDateWhen { false }
}

/** Copies the native library into the jar's `native/<os>-<arch>/` directory. */
val copyNativeLibrary by tasks.registering(Copy::class) {
    group = "build"
    description = "Copies the JNI library into the jar's native resource directory."
    dependsOn(cargoBuild)
    from(cargoTargetDir.file(nativeLibraryName))
    into(layout.buildDirectory.dir("native-resources/native/$nativePlatform"))
}

sourceSets.named("main") {
    if (prebuiltNatives != null) {
        output.dir(file(prebuiltNatives))
    } else if (testNativeLibrary == null) {
        output.dir(
            mapOf("builtBy" to copyNativeLibrary),
            layout.buildDirectory.dir("native-resources"),
        )
    }
}

// The same inputs give a byte-identical jar, so the release can check that the jar it publishes
// is the one it tested.
tasks.withType<AbstractArchiveTask>().configureEach {
    isPreserveFileTimestamps = false
    isReproducibleFileOrder = true
}

// Both licenses ship inside the jar, with the notices for the Rust crates compiled into the
// native library.
tasks.jar {
    from(files("LICENSE-MIT", "LICENSE-APACHE", "THIRD-PARTY-NOTICES.md")) {
        into("META-INF")
    }
}

tasks.test {
    useJUnitPlatform()

    // Load the library straight from a file, because the jar has not been built yet.
    if (testNativeLibrary != null) {
        systemProperty("pzmap.native.library", file(testNativeLibrary).absolutePath)
    } else {
        dependsOn(cargoBuild)
        systemProperty(
            "pzmap.native.library",
            cargoTargetDir.file(nativeLibraryName).asFile.absolutePath,
        )
    }

    // From JDK 24, loading a native library is a restricted operation.
    jvmArgs("--enable-native-access=ALL-UNNAMED")

    // The tests that read real game files use this. Without it, they are skipped.
    System.getenv("PZ_INSTALL_DIR")?.let { environment("PZ_INSTALL_DIR", it) }

    testLogging { showStandardStreams = true }
}

tasks.javadoc {
    (options as StandardJavadocDocletOptions).addStringOption("Xdoclint:none", "-quiet")
}

// Uploads to the Central Portal with `./gradlew publishToMavenCentral`. The deployment then waits
// there until someone clicks "Publish" on https://central.sonatype.com/publishing/deployments.
// The credentials and the signing key come from Gradle properties, which the release workflow
// sets from its secrets.
mavenPublishing {
    configure(JavaLibrary(javadocJar = JavadocJar.Javadoc(), sourcesJar = SourcesJar.Sources()))
    publishToMavenCentral()
    // Maven Central needs every file signed. Without a key, as in a local build, publications
    // are left unsigned, so `publishToMavenLocal` still works.
    if (providers.gradleProperty("signingInMemoryKey").isPresent) signAllPublications()
    coordinates(group.toString(), "pzmap", version.toString())
    pom {
        name = "pzmap"
        description = "Parsers for Project Zomboid's on-disk map formats."
        url = "https://github.com/LtdJorge/pzmap-rs"
        inceptionYear = "2026"
        // Dual-licensed: users may choose either license.
        licenses {
            license {
                name = "MIT License"
                url = "https://opensource.org/licenses/MIT"
            }
            license {
                name = "Apache License, Version 2.0"
                url = "https://www.apache.org/licenses/LICENSE-2.0.txt"
            }
        }
        developers {
            developer {
                id = "LtdJorge"
                name = "Jorge López"
                url = "https://github.com/LtdJorge"
            }
        }
        scm {
            url = "https://github.com/LtdJorge/pzmap-rs"
            connection = "scm:git:https://github.com/LtdJorge/pzmap-rs.git"
            developerConnection = "scm:git:ssh://git@github.com/LtdJorge/pzmap-rs.git"
        }
    }
}
