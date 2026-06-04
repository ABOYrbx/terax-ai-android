package app.crynta.terax

import android.content.Context
import java.io.File
import java.io.InputStream
import java.io.OutputStream
import java.util.concurrent.ConcurrentLinkedQueue
import java.util.concurrent.ExecutorService
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicInteger
import java.util.concurrent.atomic.AtomicReference

/**
 * Interactive terminal session that wraps a long-running process with
 * non-blocking, asynchronous stream I/O using dedicated reader/writer
 * threads.
 *
 * This is the Android-side counterpart to the Rust PTY session
 * (`session.rs` in the Tauri backend). While the desktop backend creates
 * real Unix PTY pairs via `portable-pty`, Android shell sessions
 * (including PRoot environments) use [ProcessBuilder] with concurrent
 * stream draining. The PRoot `--pty` flag or `script -q -c` can be used
 * to emulate a PTY for the child process when needed.
 *
 * Architecture
 * ------------
 * ```
 *  User keystrokes ──> write(input) ──> process.outputStream (stdin)
 *                                       process.inputStream  (stdout) ──> onOutput
 *                                       process.errorStream  (stderr) ──> onError
 *
 *  Terminal resize ──> resize(c, r) ──> stty / TIOCSWINSZ ──> process
 *  Session close   ──> stop()       ──> destroyForcibly() ──> onExit
 * ```
 *
 * Stream safety
 * -------------
 * - stdout and stderr are drained simultaneously via a dedicated thread
 *   pool (2 threads) so neither pipe fills its 64 KiB kernel buffer and
 *   deadlocks the process.
 * - stdin writes are serialized through a [ConcurrentLinkedQueue] and
 *   drained by a single writer thread, preventing interleaved writes.
 *
 * Anti-patterns this prevents
 * ----------------------------
 * - NEVER call [Process.getOutputStream().write()] from multiple threads
 *   without serialization — the JVM interleaves the byte sequences and
 *   corrupts the shell input stream.
 * - NEVER drain stdout/stderr sequentially on the same thread — a child
 *   process that writes 64 KiB+ to one pipe while the other pipe is being
 *   drained will block forever if the un-drained pipe fills up.
 * - NEVER use [Runtime.getRuntime().exec] — it does not let you set the
 *   working directory or control the environment block precisely.
 * - NEVER wrap the process streams in a buffered reader/writer from
 *   multiple threads — [BufferedReader] owns internal state and is not
 *   thread-safe.
 *
 * Usage:
 * ```kotlin
 * val session = TerminalSession(context, shell = "bash", shellArgs = listOf("-l"))
 * session.onOutput = { bytes -> channel.send(bytes) }
 * session.onExit = { code -> cleanup(code) }
 * session.start()
 * session.write("ls -la\n".toByteArray())
 * // ... later
 * session.stop()
 * ```
 */
class TerminalSession(
    private val context: Context,
    private val shell: String = "bash",
    private val shellArgs: List<String> = listOf("-l"),
    private val cwd: File? = null,
    private val environment: Map<String, String>? = null,
) {

    // ------------------------------------------------------------------
    // Observable state
    // ------------------------------------------------------------------

    /**
     * Invoked whenever new bytes arrive on the process's stdout stream.
     * Called from the background reader thread — implementations must
     * dispatch to their target thread (e.g. Android main thread via
     * [Handler], Tauri channel, or coroutine channel) without blocking.
     */
    var onOutput: ((ByteArray) -> Unit)? = null

    /**
     * Invoked whenever new bytes arrive on the process's stderr stream.
     * Same threading constraints as [onOutput].
     */
    var onError: ((ByteArray) -> Unit)? = null

    /**
     * Invoked when the process exits, with the exit code. Guaranteed to
     * fire exactly once per session, even if [stop] was called.
     */
    var onExit: ((Int) -> Unit)? = null

    /**
     * Invoked when the process starts successfully. Receives the PID
     * (or 0 if the PID is unavailable on this Android version).
     */
    var onStart: ((Int) -> Unit)? = null

    // ------------------------------------------------------------------
    // Internal state
    // ------------------------------------------------------------------

    private val env by lazy { environment ?: TermuxEnvironment(context).toMap() }

    /** The underlying OS process. Null until [start] is called. */
    @Volatile
    private var process: Process? = null

    /** Guards against double-start / double-stop. */
    private val running = AtomicBoolean(false)

    /** Exit code captured by the waiter. Default -1 = did not exit normally. */
    private val exitCode = AtomicInteger(-1)

    /** Background thread pool: 2 readers (stdout, stderr) + 1 writer (stdin) + 1 waiter. */
    private var threadPool: ExecutorService? = null

    /** Thread-safe input queue: bytes to write are enqueued here and drained by the writer thread. */
    private val inputQueue: ConcurrentLinkedQueue<ByteArray> = ConcurrentLinkedQueue()

    /** Signals the writer thread to stop blocking on an empty queue. */
    private val writerDone = AtomicBoolean(false)

    /** The resolved shell binary path (set during [start]). */
    private val resolvedShell = AtomicReference<String>()

    // ------------------------------------------------------------------
    // Lifecycle
    // ------------------------------------------------------------------

    /**
     * Start the interactive session.
     *
     * Spawns the shell process, begins draining stdout/stderr concurrently,
     * starts the stdin writer thread, and spawns a waiter thread that
     * monitors the process exit.
     *
     * @throws IllegalStateException if the session is already running.
     * @throws CommandFailedException if the shell binary cannot be resolved
     *         or the process fails to start.
     */
    fun start() {
        if (!running.compareAndSet(false, true)) {
            throw IllegalStateException("TerminalSession is already running")
        }

        val env = TermuxEnvironment(context)
        env.ensureLayout()

        // 1. Resolve the shell binary.
        val resolved = resolveShell(env)
        resolvedShell.set(resolved)

        // ANTI-PATTERN WARNING:
        //   We must fix permissions on every ancestor directory up to
        //   filesDir, not just the binary.  See TermuxExecutor.fixPathPermissions
        //   for the root cause (parent directory search bit loss).
        fixParentDirPermissions(resolved)

        // 2. Build the command.
        val cmd = mutableListOf(resolved)
        cmd.addAll(shellArgs)

        val pb = ProcessBuilder(cmd)
            .apply {
                directory(cwd ?: env.home)

                // Keep stdout/stderr separate so the UI can differentiate
                // between normal output and error diagnostics.
                redirectErrorStream(false)

                // Apply Termux environment + any caller overrides.
                environment().putAll(this@TerminalSession.env)
            }

        // 3. Start the process.
        val proc: Process = try {
            pb.start()
        } catch (e: SecurityException) {
            running.set(false)
            throw TermuxExecutor.CommandFailedException(
                error = TermuxExecutor.Error.PermissionDenied(resolved, e.message ?: ""),
                message = "Permission denied starting interactive shell: $resolved",
                cause = e,
            )
        } catch (e: Exception) {
            running.set(false)
            throw TermuxExecutor.CommandFailedException(
                error = TermuxExecutor.Error.IoFailure(e.message ?: ""),
                message = "Failed to start interactive shell: $resolved",
                cause = e,
            )
        }

        this.process = proc
        val pid = pidOf(proc)
        onStart?.invoke(pid)

        // 4. Drain stdout/stderr concurrently.
        //
        // ROOT CAUSE OF DEADLOCK:
        //   Linux pipe buffers are ~64 KiB. If the child writes more than
        //   this to stdout while we are draining stderr (or vice-versa),
        //   the write blocks forever because nobody is reading the other
        //   pipe.  Concurrent draining via a 2-thread pool eliminates this.
        val pool = Executors.newFixedThreadPool(4, NamedThreadFactory("terax-session"))
        this.threadPool = pool

        pool.submit(ReaderTask(proc.inputStream, "stdout") { bytes ->
            onOutput?.invoke(bytes)
        })
        pool.submit(ReaderTask(proc.errorStream, "stderr") { bytes ->
            onError?.invoke(bytes)
        })

        // 5. Writer thread: drain inputQueue and write to process stdin.
        pool.submit {
            val stdin: OutputStream = proc.outputStream
            try {
                while (!writerDone.get() || !inputQueue.isEmpty()) {
                    val chunk = inputQueue.poll()
                    if (chunk != null) {
                        try {
                            stdin.write(chunk)
                            stdin.flush()
                        } catch (e: Exception) {
                            // Process may have exited; stop draining.
                            break
                        }
                    } else {
                        // Sleep briefly instead of busy-waiting.
                        Thread.sleep(10)
                    }
                }
            } finally {
                try {
                    stdin.close()
                } catch (_: Exception) { }
            }
        }

        // 6. Waiter thread: wait for exit, then notify.
        pool.submit {
            val code: Int = try {
                proc.waitFor()
            } catch (e: InterruptedException) {
                proc.destroyForcibly()
                -1
            }
            exitCode.set(code)

            // Signal the writer thread to stop — without this, it keeps
            // sleeping on an empty queue after the process has exited.
            writerDone.set(true)

            // Give reader threads a moment to drain the last bytes.
            pool.shutdown()
            try {
                pool.awaitTermination(2, TimeUnit.SECONDS)
            } catch (_: InterruptedException) {
                pool.shutdownNow()
            }

            running.set(false)
            onExit?.invoke(code)
        }
    }

    /**
     * Write raw bytes to the process's stdin (the input side of the shell).
     *
     * Thread-safe. Bytes are enqueued and written sequentially by the
     * writer thread, so concurrent callers never interleave their data.
     *
     * Typical usage for user keystrokes:
     * ```kotlin
     * session.write("\r".toByteArray())        // Enter key
     * session.write("\u0003".toByteArray())     // Ctrl-C
     * session.write("ls -la\r".toByteArray())   // Command
     * ```
     *
     * For control characters, pass the raw byte value. Common ones:
     * - Ctrl-C:  `0x03`
     * - Ctrl-D:  `0x04`
     * - Ctrl-Z:  `0x1A`
     * - Enter:   `0x0D` (CR) — NOT `0x0A` (LF).  Android shell (toybox ash)
     *   expects CRLF line endings; sending bare LF can cause display issues.
     * - Backspace: `0x7F` or `0x08`
     * - Escape:  `0x1B`
     *
     * @param data Raw bytes to write.  For text input, convert with
     *             `toByteArray(Charsets.UTF_8)`.  For control bytes, use
     *             `byteArrayOf(0x03)` etc.
     */
    fun write(data: ByteArray) {
        if (!running.get()) return
        inputQueue.add(data)
    }

    /**
     * Convenience: write a UTF-8 string to the process's stdin.
     * Equivalent to `write(data.toByteArray(Charsets.UTF_8))`.
     */
    fun write(data: String) {
        write(data.toByteArray(Charsets.UTF_8))
    }

    /**
     * Resize the terminal.
     *
     * Sends `stty cols <cols> rows <rows>` to the shell to update the
     * terminal size state.  This is a best-effort operation — programs
     * running inside the shell that use ncurses / readline will respond to
     * SIGWINCH only when the controlling PTY sends the signal.  Since
     * [ProcessBuilder] creates pipes (not a PTY), SIGWINCH is not
     * delivered automatically.
     *
     * For full PTY-aware resize (including SIGWINCH delivery), use one of:
     * - PRoot with `--pty` flag, which forwards TIOCSWINSZ ioctl to the
     *   child process's controlling terminal.
     * - The Rust PTY backend (`portable-pty` via Tauri commands), which
     *   creates a real Unix PTY pair and supports [MasterPty.resize].
     *
     * Both approaches are compatible with this session class — set the
     * shell argument to the PRoot wrapper binary:
     * ```kotlin
     * TerminalSession(
     *     context,
     *     shell = "proot",
     *     shellArgs = listOf("--pty", "-0", "bash", "-l"),
     * )
     * ```
     *
     * @param cols Number of columns (width).
     * @param rows Number of rows (height).
     */
    fun resize(cols: Int, rows: Int) {
        if (!running.get()) return
        val cmd = "stty cols $cols rows $rows 2>/dev/null\n"
        write(cmd.toByteArray(Charsets.UTF_8))
    }

    /**
     * Stop the session and destroy the underlying process.
     *
     * Safe to call multiple times — subsequent calls are no-ops.
     * [onExit] is fired by the waiter thread when the process actually
     * dies, so callers should not expect [onExit] to fire synchronously
     * from within [stop].
     */
    fun stop() {
        if (!running.compareAndSet(true, false)) return
        writerDone.set(true)
        val proc = process
        if (proc != null) {
            try {
                proc.destroyForcibly()
            } catch (_: Exception) { }
        }
        // The waiter thread (submitted in start()) is blocked on
        // proc.waitFor().  destroyForcibly() causes waitFor() to return
        // immediately, and the waiter handles pool shutdown + onExit.
    }

    /**
     * True while the session is active (between [start] and [stop]/exit).
     */
    val isRunning: Boolean get() = running.get()

    /**
     * The exit code, or -1 if the process is still running or was killed.
     */
    val exitValue: Int get() = exitCode.get()

    /**
     * The resolved shell binary path (e.g. `/data/data/.../files/usr/bin/bash`).
     * Null until [start] is called.
     */
    val shellPath: String? get() = resolvedShell.get()

    // ------------------------------------------------------------------
    // Internal helpers
    // ------------------------------------------------------------------

    /**
     * Search for the shell binary: first [$PREFIX/bin], then system paths.
     *
     * Mirrors [TermuxExecutor.resolveBinary] but returns the absolute
     * path string rather than a [File], since the shell might be an
     * absolute path like `/system/bin/sh`.
     */
    private fun resolveShell(env: TermuxEnvironment): String {
        // If the shell already contains a separator, use it literally.
        if (shell.contains(File.separator)) {
            val f = File(shell)
            if (f.isFile) return f.absolutePath
            throw TermuxExecutor.CommandFailedException(
                error = TermuxExecutor.Error.NotFound(shell),
                message = "Shell binary not found at literal path: $shell",
            )
        }

        // Search $PREFIX/bin first, then system PATH.
        val searchPaths = listOf(env.binDir) + System.getenv("PATH")
            ?.split(File.pathSeparator)
            ?.map { File(it) }
            .orEmpty()

        for (dir in searchPaths) {
            val candidate = File(dir, shell)
            if (candidate.isFile) {
                return candidate.absolutePath
            }
        }

        // Fallback: try /system/bin/sh (always present on Android).
        val fallback = File("/system/bin/sh")
        if (fallback.isFile) return fallback.absolutePath

        throw TermuxExecutor.CommandFailedException(
            error = TermuxExecutor.Error.NotFound(shell),
            message = "Shell '$shell' not found in PATH or \$PREFIX/bin, " +
                "and /system/bin/sh is unavailable",
        )
    }

    /**
     * Walk from the shell binary's parent directory up to [context.filesDir]
     * and ensure every directory has the search bit (+x).  Without this,
     * execve() fails with EACCES.
     *
     * See [TermuxExecutor.fixPathPermissions] for the full root-cause analysis.
     */
    private fun fixParentDirPermissions(binary: String) {
        val file = File(binary)
        if (!file.isAbsolute) return

        var dir: File? = file.parentFile
        while (dir != null) {
            TermuxFileUtils.ensureDirMode(dir)
            if (dir == context.filesDir) break
            dir = dir.parentFile
        }

        if (file.exists()) {
            TermuxFileUtils.ensureExecutable(file)
        }
    }

    companion object {
        /**
         * Attempt to extract the PID of a [Process] via reflection.
         * Returns 0 if the PID is unavailable (pre-API 26 or restricted).
         */
        private fun pidOf(process: Process): Int {
            return try {
                val pidField = process.javaClass.getDeclaredField("pid")
                pidField.isAccessible = true
                pidField.getInt(process)
            } catch (_: Exception) {
                0
            }
        }
    }

    // ------------------------------------------------------------------
    // Internal classes
    // ------------------------------------------------------------------

    /**
     * Drains a single [InputStream] in a loop, splitting at configurable
     * chunk boundaries, and forwarding each chunk to [onChunk].
     *
     * This is the equivalent of the Rust reader thread in `session.rs`
     * but without the DA filter or agent detector (those run on the Rust
     * side for desktop; on Android they can be added by wrapping
     * [onOutput] / [onError] callbacks if needed).
     */
    private class ReaderTask(
        private val stream: InputStream,
        private val label: String,
        private val onChunk: (ByteArray) -> Unit,
    ) : Runnable {

        companion object {
            /** Same buffer size as the Rust reader in session.rs. */
            private const val READ_BUF = 16 * 1024
        }

        override fun run() {
            val buf = ByteArray(READ_BUF)
            try {
                while (true) {
                    val n = stream.read(buf)
                    if (n == -1) break // EOF
                    if (n > 0) {
                        val chunk = if (n == buf.size) buf else buf.copyOf(n)
                        onChunk(chunk)
                    }
                }
            } catch (e: Exception) {
                // Stream closed or process died — normal termination.
            } finally {
                try {
                    stream.close()
                } catch (_: Exception) { }
            }
        }
    }
}

/**
 * [ThreadFactory] that names daemon threads for [TerminalSession].
 * Makes thread dumps readable when debugging deadlocks.
 */
private class NamedThreadFactory(private val prefix: String) : java.util.concurrent.ThreadFactory {
    private val count = AtomicInteger(0)

    override fun newThread(r: Runnable): Thread {
        val t = Thread(r, "$prefix-${count.incrementAndGet()}")
        t.isDaemon = true
        return t
    }
}
