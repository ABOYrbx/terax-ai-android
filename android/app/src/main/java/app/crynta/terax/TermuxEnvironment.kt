package app.crynta.terax

import android.content.Context
import java.io.File

/**
 * Builds the environment block that must be passed to [ProcessBuilder] so
 * Termux binaries can find their dependencies inside the app sandbox
 * instead of resolving to system paths.
 *
 * Architecture
 * ------------
 * Termux replicates a Linux rootfs under [PREFIX] (usr/):
 *   files/
 *     home/          ← $HOME
 *     usr/           ← $PREFIX
 *       bin/         ← $PATH (prepended)
 *       lib/         ← $LD_LIBRARY_PATH
 *       etc/
 *       var/
 *       tmp/         ← $TMPDIR
 *     tmp/           ← also $TMPDIR fallback
 *
 * For every [ProcessBuilder.start()] or [Runtime.exec()] call, you MUST:
 * 1. Call [toMap] and pass the result to [ProcessBuilder.environment()].
 * 2. Call [ProcessBuilder.directory] with [home] as the working directory.
 * 3. Call [inheritIO] or pipe stdout/stderr explicitly.
 */
class TermuxEnvironment(private val context: Context) {

    /** $HOME — the user's home directory inside the sandbox. */
    val home: File get() = File(context.filesDir, "home")

    /** $PREFIX — the Termux-style prefix (usr/). */
    val prefix: File get() = File(context.filesDir, "usr")

    /** $PREFIX/bin — where Termux packages install executables. */
    val binDir: File get() = File(prefix, "bin")

    /** $PREFIX/lib — where shared libraries (libc, libssl, ...) live. */
    val libDir: File get() = File(prefix, "lib")

    /** $TMPDIR — writable temp directory. */
    val tmpDir: File get() = File(prefix, "tmp")

    /**
     * Build the complete environment map.
     *
     * - [PATH]: puts [$PREFIX/bin] first so Termux binaries shadow system ones.
     * - [LD_LIBRARY_PATH]: points at [$PREFIX/lib] so `apt`, `dpkg`, etc.
     *   find their .so dependencies.
     * - [HOME]: the user's home inside the sandbox.
     * - [PREFIX]: standard Termux env var.
     * - [TMPDIR]: required by `apt` and `dpkg` for transient files.
     * - [TERM]: forces terminfo resolution inside the sandbox.
     * - [ENV] / [BASH_ENV]: point at `$HOME/.shrc` so every `sh -c` / `bash -c`
     *   invocation sources the permission-repair script before running.
     * - [TERAX_HOME] / [TERAX_PREFIX]: consumed by the Rust backend for
     *   filesystem layout resolution.
     *
     * Inherits the current process environment and overrides the critical vars.
     */
    fun toMap(): Map<String, String> {
        val currentEnv = System.getenv()
        val env = mutableMapOf<String, String>()

        // Preserve the existing environment, but override our critical vars.
        env.putAll(currentEnv)

        env["HOME"] = home.absolutePath
        env["PREFIX"] = prefix.absolutePath
        env["TMPDIR"] = tmpDir.absolutePath
        env["TERM"] = "xterm-256color"
        env["LD_LIBRARY_PATH"] = libDir.absolutePath
        env["PATH"] = buildPath(currentEnv["PATH"])

        // Tell the shell to source .shrc for permission repair + env init.
        // Without these, sh -c / bash -c invocations skip the rcfile
        // and every command in $PREFIX/bin yields EACCES.
        val shrc = home.resolve(".shrc")
        if (shrc.exists()) {
            env["ENV"] = shrc.absolutePath
            env["BASH_ENV"] = shrc.absolutePath
        }

        // Rust backend compat — TERAX.md documents these as the canonical
        // Android paths; the Rust android_fs module uses OnceLock'd globals
        // but also reads these from the process environment if set.
        env["TERAX_HOME"] = home.absolutePath
        env["TERAX_PREFIX"] = prefix.absolutePath

        return env
    }

    /**
     * Returns the PATH string with [$PREFIX/bin] first, followed by
     * standard Android system paths, then the inherited PATH.
     */
    private fun buildPath(inheritedPath: String?): String {
        val parts = mutableListOf(
            binDir.absolutePath,
            "/system/bin",
            "/system/xbin",
            "/vendor/bin",
            "/product/bin",
        )
        inheritedPath?.split(File.pathSeparator)?.let { systemPaths ->
            for (p in systemPaths) {
                if (p !in parts) {
                    parts.add(p)
                }
            }
        }
        return parts.joinToString(File.pathSeparator)
    }

    /**
     * Ensure the standard directory layout exists. Call once during app init.
     */
    fun ensureLayout() {
        home.mkdirs()
        binDir.mkdirs()
        libDir.mkdirs()
        tmpDir.mkdirs()

        TermuxFileUtils.ensureDirMode(context.filesDir)
        TermuxFileUtils.ensureDirMode(home)
        TermuxFileUtils.ensureDirMode(prefix)
        TermuxFileUtils.ensureDirMode(binDir)
        TermuxFileUtils.ensureDirMode(tmpDir)
    }
}
