package app.crynta.terax

import android.content.Context
import java.io.File
import java.io.InputStream
import java.util.concurrent.Callable
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit

class ProotExecutor(private val context: Context) {

    private val prootEnv by lazy { ProotEnvironment(context) }
    private val envManager by lazy { EnvironmentManager(context) }

    fun runInProot(
        guestCommand: String,
        guestArgs: List<String> = emptyList(),
        rootfs: File,
        binds: List<ProotEnvironment.BindMount> = prootEnv.mandatoryBinds + prootEnv.optionalBinds,
        timeoutSecs: Long = 60,
        guestCwd: String = "/root",
    ): TermuxExecutor.Result {
        prootEnv.ensureLayout()

        val binary = prootEnv.prootBinary
        if (!binary.isFile) {
            throw TermuxExecutor.CommandFailedException(
                TermuxExecutor.Error.NotFound(binary.absolutePath),
                "PRoot binary not found at: ${binary.absolutePath}. " +
                    "Extract the proot-${prootEnv.hostArch} binary to ${prootEnv.prootBinary.absolutePath} " +
                    "or ensure it is on the Termux PATH.",
            )
        }

        fixPathPermissions(binary)

        val cmd = prootEnv.buildProotCommand(rootfs, guestCommand, guestArgs, binds, guestCwd)
        return runCommand(cmd, timeoutSecs)
    }

    fun runInProot(
        guestCommand: String,
        guestArgs: List<String> = emptyList(),
        envId: String,
        binds: List<ProotEnvironment.BindMount> = prootEnv.mandatoryBinds + prootEnv.optionalBinds,
        timeoutSecs: Long = 60,
        guestCwd: String = "/root",
    ): TermuxExecutor.Result {
        val entry = envManager.get(envId)
            ?: throw TermuxExecutor.CommandFailedException(
                TermuxExecutor.Error.NotFound(envId),
                "Environment not found: $envId. Use 'env list' to see available environments.",
            )
        return runInProot(
            guestCommand = guestCommand,
            guestArgs = guestArgs,
            rootfs = entry.rootfsDir(context),
            binds = binds,
            timeoutSecs = timeoutSecs,
            guestCwd = guestCwd,
        )
    }

    fun shellInProot(
        script: String,
        rootfs: File,
        binds: List<ProotEnvironment.BindMount> = prootEnv.mandatoryBinds + prootEnv.optionalBinds,
        timeoutSecs: Long = 60,
        guestCwd: String = "/root",
    ): TermuxExecutor.Result {
        return runInProot(
            guestCommand = "/bin/sh",
            guestArgs = listOf("-c", script),
            rootfs = rootfs,
            binds = binds,
            timeoutSecs = timeoutSecs,
            guestCwd = guestCwd,
        )
    }

    fun shellInProot(
        script: String,
        envId: String,
        binds: List<ProotEnvironment.BindMount> = prootEnv.mandatoryBinds + prootEnv.optionalBinds,
        timeoutSecs: Long = 60,
        guestCwd: String = "/root",
    ): TermuxExecutor.Result {
        return runInProot(
            guestCommand = "/bin/sh",
            guestArgs = listOf("-c", script),
            envId = envId,
            binds = binds,
            timeoutSecs = timeoutSecs,
            guestCwd = guestCwd,
        )
    }

    fun launchDefault(
        guestCommand: String,
        guestArgs: List<String> = emptyList(),
        timeoutSecs: Long = 60,
    ): TermuxExecutor.Result {
        val default = envManager.bootTarget()
            ?: throw TermuxExecutor.CommandFailedException(
                TermuxExecutor.Error.NotFound("(default)"),
                "No environments installed and no default set. " +
                    "Register one with 'env create'.",
            )
        return runInProot(
            guestCommand = guestCommand,
            guestArgs = guestArgs,
            envId = default.id,
            timeoutSecs = timeoutSecs,
        )
    }

    fun spawnBackgroundInProot(
        guestCommand: String,
        guestArgs: List<String> = emptyList(),
        rootfs: File,
        binds: List<ProotEnvironment.BindMount> = prootEnv.mandatoryBinds + prootEnv.optionalBinds,
        guestCwd: String = "/root",
    ): Process {
        prootEnv.ensureLayout()

        val binary = prootEnv.prootBinary
        fixPathPermissions(binary)

        val cmd = prootEnv.buildProotCommand(rootfs, guestCommand, guestArgs, binds, guestCwd)
        val pb = ProcessBuilder(cmd)
            .apply {
                directory(rootfs)
                environment().clear()
                environment().putAll(prootEnv.toPristineEnv())
                redirectErrorStream(true)
            }

        return pb.start()
    }

    fun spawnBackgroundInProot(
        guestCommand: String,
        guestArgs: List<String> = emptyList(),
        envId: String,
        binds: List<ProotEnvironment.BindMount> = prootEnv.mandatoryBinds + prootEnv.optionalBinds,
        guestCwd: String = "/root",
        track: Boolean = true,
    ): Process {
        val entry = envManager.get(envId)
            ?: throw TermuxExecutor.CommandFailedException(
                TermuxExecutor.Error.NotFound(envId),
                "Environment not found: $envId.",
            )
        val proc = spawnBackgroundInProot(
            guestCommand = guestCommand,
            guestArgs = guestArgs,
            rootfs = entry.rootfsDir(context),
            binds = binds,
            guestCwd = guestCwd,
        )
        if (track) {
            envManager.markRunning(envId, proc)
        }
        return proc
    }

    private fun runCommand(
        cmd: List<String>,
        timeoutSecs: Long,
    ): TermuxExecutor.Result {
        val pb = ProcessBuilder(cmd)
            .apply {
                directory(prootEnv.environmentsBase)

                redirectErrorStream(false)

                environment().clear()
                environment().putAll(prootEnv.toPristineEnv())
            }

        val process: Process
        try {
            process = pb.start()
        } catch (e: SecurityException) {
            throw TermuxExecutor.CommandFailedException(
                TermuxExecutor.Error.PermissionDenied(cmd[0], e.message ?: ""),
                "Permission denied executing proot: ${cmd[0]}. " +
                    "Ensure proot binary has +x and is in a non-noexec filesystem.",
                e,
            )
        } catch (e: Exception) {
            throw TermuxExecutor.CommandFailedException(
                TermuxExecutor.Error.IoFailure(e.message ?: ""),
                "Failed to start proot: ${cmd.joinToString(" ")}",
                e,
            )
        }

        val drainPool = Executors.newFixedThreadPool(2)
        val stdoutFuture = drainPool.submit(Callable { readStream(process.inputStream) })
        val stderrFuture = drainPool.submit(Callable { readStream(process.errorStream) })
        drainPool.shutdown()

        val finished: Boolean
        try {
            finished = process.waitFor(timeoutSecs, TimeUnit.SECONDS)
        } catch (e: InterruptedException) {
            process.destroyForcibly()
            throw TermuxExecutor.CommandFailedException(
                TermuxExecutor.Error.Timeout(cmd.joinToString(" "), timeoutSecs),
                "Proot command interrupted: ${cmd.joinToString(" ")}",
            )
        }

        val stdout = try { stdoutFuture.get(5, TimeUnit.SECONDS) } catch (_: Exception) { "" }
        val stderr = try { stderrFuture.get(5, TimeUnit.SECONDS) } catch (_: Exception) { "" }

        if (!finished) {
            process.destroyForcibly()
            throw TermuxExecutor.CommandFailedException(
                TermuxExecutor.Error.Timeout(cmd.joinToString(" "), timeoutSecs),
                "Proot command timed out after ${timeoutSecs}s: ${cmd.joinToString(" ")}",
            )
        }

        val exitCode = process.exitValue()

        return TermuxExecutor.Result(
            exitCode = exitCode,
            stdout = stdout,
            stderr = stderr,
            command = cmd.joinToString(" "),
        )
    }

    private fun fixPathPermissions(binary: File) {
        if (!binary.isAbsolute) return

        var dir: File? = binary.parentFile
        while (dir != null) {
            TermuxFileUtils.ensureDirMode(dir)
            if (dir == context.filesDir) break
            dir = dir.parentFile
        }

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
}
