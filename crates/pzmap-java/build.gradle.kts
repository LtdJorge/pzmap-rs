import org.gradle.internal.os.OperatingSystem

plugins {
    `java-library`
    `maven-publish`
}

java {
    // The API uses records, and 17 is the oldest long-term-support Java release that has them.
    toolchain { languageVersion = JavaLanguageVersion.of(17) }
    withSourcesJar()
    withJavadocJar()
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
    output.dir(mapOf("builtBy" to copyNativeLibrary), layout.buildDirectory.dir("native-resources"))
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
    dependsOn(cargoBuild)

    // Load the library straight from cargo's output, because the jar has not been built yet.
    systemProperty(
        "pzmap.native.library",
        cargoTargetDir.file(nativeLibraryName).asFile.absolutePath,
    )

    // From JDK 24, loading a native library is a restricted operation.
    jvmArgs("--enable-native-access=ALL-UNNAMED")

    // The tests that read real game files use this. Without it, they are skipped.
    System.getenv("PZ_INSTALL_DIR")?.let { environment("PZ_INSTALL_DIR", it) }

    testLogging { showStandardStreams = true }
}

tasks.javadoc {
    (options as StandardJavadocDocletOptions).addStringOption("Xdoclint:none", "-quiet")
}

publishing {
    publications {
        create<MavenPublication>("maven") {
            from(components["java"])
            artifactId = "pzmap"
            pom {
                name = "pzmap"
                description = "Parsers for Project Zomboid's on-disk map formats."
                url = "https://github.com/LtdJorge/pzmap-rs"
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
                developers { developer { name = "Jorge López" } }
                scm { url = "https://github.com/LtdJorge/pzmap-rs" }
            }
        }
    }
}
