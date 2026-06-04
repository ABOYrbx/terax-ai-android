package app.crynta.terax

import android.content.Context
import android.os.Build
import java.io.File

class ProotEnvironment(private val context: Context) {

    data class BindMount(
        val host: String,
        val guest: String = host,
    )

    companion object {
        private val ABI_TO_PROOT_ARCH = mapOf(
            "arm64-v8a" to "aarch64",
            "armeabi-v7a" to "arm",
            "armeabi" to "arm",
            "x86_64" to "x86_64",
            "x86" to "i686",
        )
    }

    val hostArch: String
        get() = Build.SUPPORTED_ABIS.firstOrNull()
            ?.let { ABI_TO_PROOT_ARCH[it] }
            ?: "aarch64"

    val prootBinary: File
        get() {
            val candidates = listOf(
                File(context.filesDir, "usr/bin/proot-$hostArch"),
                File(context.filesDir, "usr/bin/proot"),
                File(context.filesDir, "proot/proot-$hostArch"),
                File(context.filesDir, "proot/proot"),
            )
            for (c in candidates) {
                if (c.isFile) return c
            }
            return candidates[0]
        }

    val environmentsBase: File
        get() = File(context.filesDir, "environments")

    fun environmentDir(id: String): File = File(environmentsBase, id)

    val mandatoryBinds: List<BindMount>
        get() = listOf(
            BindMount("/dev"),
            BindMount("/proc"),
            BindMount("/sys"),
        )

    val optionalBinds: List<BindMount>
        get() = listOf(
            BindMount("/system"),
            BindMount("/vendor"),
            BindMount("/product"),
            BindMount("/data/data/${context.packageName}/files/home", "/host"),
        )

    fun buildProotCommand(
        rootfs: File,
        guestCommand: String,
        guestArgs: List<String> = emptyList(),
        binds: List<BindMount> = mandatoryBinds + optionalBinds,
        guestCwd: String = "/root",
    ): List<String> {
        val cmd = mutableListOf(prootBinary.absolutePath)
        cmd.add("-r")
        cmd.add(rootfs.absolutePath)
        cmd.add("-w")
        cmd.add(guestCwd)
        cmd.add("-0")

        for (b in binds) {
            cmd.add("-b")
            cmd.add("${b.host}:${b.guest}")
        }

        cmd.add(guestCommand)
        cmd.addAll(guestArgs)
        return cmd
    }

    fun buildProotCommand(
        envId: String,
        guestCommand: String,
        guestArgs: List<String> = emptyList(),
        binds: List<BindMount> = mandatoryBinds + optionalBinds,
        guestCwd: String = "/root",
    ): List<String> {
        val rootfs = environmentDir(envId)
        return buildProotCommand(rootfs, guestCommand, guestArgs, binds, guestCwd)
    }

    fun toPristineEnv(): Map<String, String> {
        return mapOf(
            "PATH" to "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin",
            "HOME" to "/root",
            "USER" to "root",
            "TERM" to "xterm-256color",
            "LD_LIBRARY_PATH" to "/usr/lib:/lib",
            "SHELL" to "/bin/bash",
            "TMPDIR" to "/tmp",
            "CONSOLE" to "linux",
            "TERMUX_VERSION" to "",
        )
    }

    fun ensureLayout() {
        environmentsBase.mkdirs()
        TermuxFileUtils.ensureDirMode(environmentsBase)
    }
}
