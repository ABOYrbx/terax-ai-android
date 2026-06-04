package app.crynta.terax

import android.content.Context
import java.io.File
import java.io.InputStream
import java.util.concurrent.Callable
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit

/**
 * Safe wrapper around [ProcessBuilder] for executing binaries inside the
 * Android sandbox.  Automatically applies:
 *
 * 1. The correct working directory ([TermuxEnvironment.home]).
 * 2. The environment block ([TermuxEnvironment.toMap]).
 * 3. Error classification for EACCES / ENOENT so you can diagnose failures.
 *
 * Anti-patterns this prevents:
 * - NEVER use [Runtime.getRuntime().exec] — it silently swallows env vars
 *   and does not let you set the working directory.
 * - NEVER inherit the system PATH without prepending [$PREFIX/bin].
 * - NEVER execute files from /sdcard or public dirs (they are [noexec]).
 * - NEVER forget [ProcessBuilder.redirectErrorStream(true)] — without it,
 *   stderr fills a tiny buffer and deadlocks the process.
 *
 * Usage:
 * ```kotlin
 * val executor = TermuxExecutor(context)
 * val result = executor.run("pkg", listOf("install", "git"), timeoutSecs = 30)
 * ```
 */
class TermuxExecutor(private val context: Context) {

    data class Result(
        val exitCode: Int,
        val stdout: String,
        val stderr: String,
        val command: String,
    ) {
        val isSuccess: Boolean get() = exitCode == 0
    }

    sealed class Error {
        data class PermissionDenied(val path: String, val detail: String) : Error()
        data class NotFound(val path: String) : Error()
        data class IoFailure(val detail: String) : Error()
        data class Timeout(val command: String, val timeoutSecs: Long) : Error()
        data class ExitCode(val command: String, val code: Int, val stderr: String) : Error()
    }

    private val env by lazy { TermuxEnvironment(context) }

    /**
     * Run a command synchronously and capture stdout + stderr.
     *
     * @param command    The command to run (e.g. "pkg" or "bash").
     * @param args       Arguments to the command.
     * @param timeoutSecs Max execution time before the process is destroyed.
     * @param cwd        Working directory (defaults to [TermuxEnvironment.home]).
     * @return [Result] on success, or throws [CommandFailedException] on failure.
     */
    fun run(
        command: String,
        args: List<String> = emptyList(),
        timeoutSecs: Long = 60,
        cwd: File? = null,
    ): Result {
        return runCommand(command, args, timeoutSecs, cwd)
    }

    /**
     * Run a shell command via `/system/bin/sh -l -c <command>`.
     * The `-l` (login shell) flag sources `.profile` -> `.shrc` which
     * fixes execute permissions and sets up the environment.  On toybox
     * ash (default `sh` on modern Android), `$ENV` is NOT supported for
     * non-interactive shells, so `-l` is the only reliable init path.
     * Useful for pipelines, redirection, or when the binary needs shell features.
     */
    fun shell(
        script: String,
        timeoutSecs: Long = 60,
        cwd: File? = null,
    ): Result {
        // Permission pre-flight: ensure every directory from context.filesDir
        // through $PREFIX/bin has the search bit (+x).  Without this, the
        // login shell (-l) may not even start if a parent directory lost +x.
        env.ensureLayout()
        TermuxFileUtils.ensureDirMode(context.filesDir)
        var dir: File? = env.binDir
        while (dir != null) {
            TermuxFileUtils.ensureDirMode(dir)
            if (dir == context.filesDir) break
            dir = dir.parentFile
        }

        val shell = resolveShell()
        return runCommand(shell, listOf("-l", "-c", script), timeoutSecs, cwd)
    }

    /**
     * Resolves the best available shell: $PREFIX/bin/bash > /system/bin/bash > /system/bin/sh.
     * The parent directory is chmodded so the kernel can traverse into it.
     */
    private fun resolveShell(): String {
        val candidates = listOf(
            env.binDir.resolve("bash"),
            File("/system/bin/bash"),
            File("/system/bin/sh"),
        )
        for (c in candidates) {
            if (c.exists()) {
                TermuxFileUtils.ensureExecutable(c)
                TermuxFileUtils.ensureDirMode(c.parentFile!!)
                return c.absolutePath
            }
        }
        return "sh"
    }

    private fun runCommand(
        command: String,
        args: List<String>,
        timeoutSecs: Long,
        cwd: File?,
    ): Result {
        env.ensureLayout()

        // 0. Resolve the binary path.
        // If the command doesn't contain a '/', search $PREFIX/bin first.
        val resolved = resolveBinary(command)
            ?: throw CommandFailedException(
                Error.NotFound(command),
                "Binary not found: $command (searched PATH including ${env.binDir})"
            )

        // 0a. Fix permissions on the binary and every ancestor directory
        //     all the way up to filesDir.
        //
        // ROOT CAUSE OF "PERMISSION DENIED" ON THE FOLDER:
        // When any ancestor directory (especially filesDir/ itself) loses
        // its search bit (+x), the Linux kernel returns EACCES during
        // execve() path resolution — even when the binary itself has +x.
        // The error message shows the binary path but the actual fault is
        // a parent directory the kernel cannot traverse through.
        fixPathPermissions(resolved)

        // 1. Build the process.
        val cmd = mutableListOf(resolved.absolutePath)
        cmd.addAll(args)

        val pb = ProcessBuilder(cmd)
            .apply {
                // ANTI-PATTERN WARNING:
                //   ProcessBuilder must use directory(), not cwd(), for
                //   the working directory.  The Java API is:
                //     pb.directory(cwd)
                //   NOT:
                //     pb.environment()["PWD"] = cwd   ← WRONG, no effect
                directory(cwd ?: env.home)

                // redirectErrorStream(true) merges stderr into stdout (simpler
                // but loses separate stderr for diagnostics).  We keep them
                // separate and drain both pipes concurrently via the executor
                // pool above — without concurrent draining, a child writing
                // >64 KiB to either pipe would deadlock (Linux pipe buffer
                // limit).
                redirectErrorStream(false)

                // 2. Apply the Termux environment block.
                // DO NOT call environment().clear() — that wipes Android's
                // essential vars (BOOTCLASSPATH, DEX2OATBOOTCLASSPATH, etc.)
                // and breaks system tooling.  Override only our own keys.
                environment().putAll(env.toMap())
            }

        // 3. Start the process.
        val process: Process
        try {
            process = pb.start()
        } catch (e: SecurityException) {
            throw CommandFailedException(
                Error.PermissionDenied(resolved.absolutePath, e.message ?: ""),
                "Permission denied executing: $command at ${resolved.absolutePath}. " +
                    "Ensure the file has +x and is in a non-noexec filesystem.",
                e,
            )
        } catch (e: Exception) {
            throw CommandFailedException(
                Error.IoFailure(e.message ?: ""),
                "Failed to start: $command",
                e,
            )
        }

        // 4. Drain stdout and stderr concurrently to prevent deadlock.
        // Without concurrent draining, a child writing 64 KiB+ to either
        // pipe blocks forever because the pipe buffer is full.
        val drainPool = Executors.newFixedThreadPool(2)
        val stdoutFuture = drainPool.submit(Callable { readStream(process.inputStream) })
        val stderrFuture = drainPool.submit(Callable { readStream(process.errorStream) })
        drainPool.shutdown()

        // 5. Wait with timeout.
        val finished: Boolean
        try {
            finished = process.waitFor(timeoutSecs, TimeUnit.SECONDS)
        } catch (e: InterruptedException) {
            process.destroyForcibly()
            throw CommandFailedException(
                Error.Timeout(command, timeoutSecs),
                "Command interrupted: $command",
            )
        }

        val stdout = try { stdoutFuture.get(5, TimeUnit.SECONDS) } catch (_: Exception) { "" }
        val stderr = try { stderrFuture.get(5, TimeUnit.SECONDS) } catch (_: Exception) { "" }

        if (!finished) {
            process.destroyForcibly()
            throw CommandFailedException(
                Error.Timeout(command, timeoutSecs),
                "Command timed out after ${timeoutSecs}s: $command",
            )
        }

        val exitCode = process.exitValue()

        return Result(
            exitCode = exitCode,
            stdout = stdout,
            stderr = stderr,
            command = "$command ${args.joinToString(" ")}",
        )
    }

    /**
     * Asynchronously run a command in the background (fire-and-forget).
     * Returns a [ProcessHandle] that can be used to check status or kill.
     */
    fun spawnBackground(
        command: String,
        args: List<String> = emptyList(),
        cwd: File? = null,
    ): Process {
        env.ensureLayout()

        val resolved = resolveBinary(command)
            ?: throw CommandFailedException(
                Error.NotFound(command),
                "Binary not found: $command",
            )

        val cmd = mutableListOf(resolved.absolutePath)
        cmd.addAll(args)

        fixPathPermissions(resolved)

        val pb = ProcessBuilder(cmd)
            .apply {
                directory(cwd ?: env.home)
                redirectErrorStream(true)
                // DO NOT clear — preserves Android's BOOTCLASSPATH, etc.
                environment().putAll(env.toMap())
            }

        return pb.start()
    }

    /**
     * Search for a binary: first [$PREFIX/bin], then system PATH.
     *
     * Unlike a naive check, this does NOT require [File.canExecute] upfront
     * — freshly-extracted files from TermuxFileUtils may exist but lack +x
     * until [TermuxFileUtils.ensureExecutable] is called.  We return the
     * first candidate that exists; the caller fixes permissions before spawn.
     */
    private fun resolveBinary(command: String): File? {
        if (command.contains(File.separator)) {
            val f = File(command)
            if (f.isFile) return f
            return null
        }

        val searchPaths = listOf(env.binDir) + System.getenv("PATH")
            ?.split(File.pathSeparator)
            ?.map { File(it) }
            .orEmpty()

        for (dir in searchPaths) {
            val candidate = File(dir, command)
            if (candidate.isFile) {
                return candidate
            }
        }
        return null
    }

    /**
     * Walk from [binary]'s parent directory up to [context.filesDir] and
     * ensure every directory has the search bit (+x).  Then fix the binary
     * itself too, in case it lost the execute bit.
     *
     * Without this, execve() fails with EACCES when any directory in the
     * resolution path lacks the search permission.  This is the #1 cause
     * of "Permission denied" errors on Android — the error message shows
     * the binary path but the real fault is a parent directory (often
     * filesDir/ itself) whose +x was stripped by backup/restore or OEM
     * "optimisations".
     */
    private fun fixPathPermissions(binary: File) {
        if (!binary.isAbsolute) return

        // Fix all ancestor directories, INCLUDING filesDir.
        var dir: File? = binary.parentFile
        while (dir != null) {
            TermuxFileUtils.ensureDirMode(dir)
            if (dir == context.filesDir) break
            dir = dir.parentFile
        }

        // Fix the binary itself.
        if (binary.exists()) {
            TermuxFileUtils.ensureExecutable(binary)
        }
    }

    private fun readStream(stream: InputStream): String {
        return try {
            stream.bufferedReader().use { it.readText() }
        } catch (_: Exception) {
            ""
        }
    }

    class CommandFailedException(
        val error: Error,
        message: String,
        cause: Throwable? = null,
    ) : RuntimeException(message, cause)
}
