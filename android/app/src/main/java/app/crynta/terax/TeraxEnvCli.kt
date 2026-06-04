package app.crynta.terax

import android.content.Context
import java.io.File

object TeraxEnvCli {

    private const val YLW = "\u001b[1;33m"
    private const val CYN = "\u001b[1;36m"
    private const val GRN = "\u001b[1;32m"
    private const val RED = "\u001b[1;31m"
    private const val WHT = "\u001b[1;37m"
    private const val RST = "\u001b[0m"
    private const val DIM = "\u001b[2m"

    fun handle(context: Context, args: List<String>): String {
        if (args.isEmpty()) return help()
        return when (args[0]) {
            "list", "ls" -> listEnvironments(context)
            "delete", "rm" -> deleteEnvironment(context, args.getOrNull(1))
            "default", "set-default" -> setDefaultEnvironment(context, args.getOrNull(1))
            "show", "info" -> showInfo(context)
            "create", "register" -> createEnvironment(context, args.drop(1))
            "config", "settings" -> showSettings(context)
            "help", "--help", "-h" -> help()
            else -> "Unknown command: ${args[0]}\n${help()}"
        }
    }

    fun help(): String = buildString {
        appendLine("${YLW}╔══════════════════════════════════════════════════════════╗${RST}")
        appendLine("${YLW}║${RST}  ${WHT}Terax Environment Manager${RST}                                ${YLW}║${RST}")
        appendLine("${YLW}╚══════════════════════════════════════════════════════════╝${RST}")
        appendLine()
        appendLine("${WHT}Usage:${RST} env <command> [arguments]")
        appendLine()
        appendLine("${WHT}Commands:${RST}")
        appendLine("  ${CYN}list${RST}        ${DIM}ls${RST}          List all installed environments")
        appendLine("  ${CYN}create${RST}     ${DIM}register${RST}    Register a new environment")
        appendLine("  ${CYN}delete${RST}     ${DIM}rm${RST}          Delete an environment (files + registry)")
        appendLine("  ${CYN}default${RST}    ${DIM}set-default${RST} Set the default boot environment")
        appendLine("  ${CYN}show${RST}       ${DIM}info${RST}        Show detailed environment information")
        appendLine("  ${CYN}config${RST}     ${DIM}settings${RST}    Show global configuration")
        appendLine("  ${CYN}help${RST}                           Show this help message")
        appendLine()
        appendLine("${WHT}Examples:${RST}")
        appendLine("  ${DIM}env list${RST}")
        appendLine("  ${DIM}env create ubuntu_prod --name \"Ubuntu Production\" --distro ubuntu${RST}")
        appendLine("  ${DIM}env delete alpine_v1${RST}")
        appendLine("  ${DIM}env default ubuntu_prod${RST}")
    }

    fun listEnvironments(context: Context): String {
        val mgr = EnvironmentManager(context)
        val settings = TeraxSettings(context)
        val data = settings.load()
        val envs = mgr.list()

        if (envs.isEmpty()) {
            return buildString {
                appendLine("${YLW}╔══════════════════════════════════════════════════════════╗${RST}")
                appendLine("${YLW}║${RST}  ${WHT}Installed Environments${RST}                                ${YLW}║${RST}")
                appendLine("${YLW}╚══════════════════════════════════════════════════════════╝${RST}")
                appendLine()
                appendLine("${DIM}No environments installed.${RST}")
                appendLine()
                appendLine("To get started, create one:")
                appendLine("  ${CYN}env create my_env --distro ubuntu${RST}")
            }
        }

        return buildString {
            appendLine("${YLW}╔══════════════════════════════════════════════════════════╗${RST}")
            appendLine("${YLW}║${RST}  ${WHT}Installed Environments${RST}                                ${YLW}║${RST}")
            appendLine("${YLW}╚══════════════════════════════════════════════════════════╝${RST}")
            appendLine()

            for (entry in envs) {
                val isDefault = entry.id == data.defaultEnvironmentId
                val running = mgr.isRunning(entry.id)
                val marker = if (isDefault) "${GRN}*${RST}" else " "
                val status = when {
                    running -> "${GRN}[running]${RST}"
                    else -> "${DIM}[stopped]${RST}"
                }
                val dir = entry.rootfsDir(context)
                val exists = if (dir.exists()) "${GRN}present${RST}" else "${RED}missing${RST}"

                appendLine("  $marker ${CYN}${entry.id}${RST}  $status  $exists")
                appendLine("       ${DIM}Name:${RST}      ${entry.name}")
                appendLine("       ${DIM}Distro:${RST}    ${entry.distro}  (${entry.arch})")
                appendLine("       ${DIM}Shell:${RST}     ${entry.shell}")
                appendLine("       ${DIM}Path:${RST}      ${dir.absolutePath}")
                if (entry.notes.isNotEmpty()) {
                    appendLine("       ${DIM}Notes:${RST}    ${entry.notes}")
                }
                appendLine()
            }

            appendLine("${DIM}${envs.size} environment(s) installed.${RST}")
            appendLine("${DIM}Settings file: ${settings.filePath()}${RST}")
        }
    }

    fun deleteEnvironment(context: Context, id: String?): String {
        if (id == null) {
            return "${RED}Error:${RST} Missing environment ID.\nUsage: env delete <id>"
        }

        val mgr = EnvironmentManager(context)
        val info = mgr.get(id)

        val msg = buildString {
            appendLine("${YLW}╔══════════════════════════════════════════════════════════╗${RST}")
            appendLine("${YLW}║${RST}  ${WHT}Delete Environment${RST}                                        ${YLW}║${RST}")
            appendLine("${YLW}╚══════════════════════════════════════════════════════════╝${RST}")
            appendLine()

            if (info != null) {
                appendLine("  ${WHT}Environment:${RST} ${CYN}${id}${RST}")
                appendLine("  ${WHT}Name:${RST}        ${info.name}")
                appendLine("  ${WHT}Distro:${RST}      ${info.distro}")
                appendLine("  ${WHT}Path:${RST}        ${info.rootfsDir(context).absolutePath}")
                appendLine()
            }

            val result = mgr.delete(id)
            when (result) {
                is EnvironmentManager.ValidationResult.Success -> {
                    appendLine("${GRN}Deleted environment: ${result.entry.id}${RST}")
                }
                is EnvironmentManager.ValidationResult.NotFound -> {
                    appendLine("${RED}Error:${RST} Environment '${id}' not found.")
                }
                is EnvironmentManager.ValidationResult.StillRunning -> {
                    appendLine("${RED}Error:${RST} Environment '${id}' is still running " +
                        "(${result.processCount} active process(es)).")
                    appendLine("Stop it before deleting.")
                }
                else -> {
                    appendLine("${RED}Error:${RST} Could not delete environment '${id}'.")
                }
            }
        }
        return msg
    }

    fun setDefaultEnvironment(context: Context, id: String?): String {
        if (id == null) {
            val mgr = EnvironmentManager(context)
            val current = mgr.getDefault()
            return buildString {
                appendLine("${YLW}╔══════════════════════════════════════════════════════════╗${RST}")
                appendLine("${YLW}║${RST}  ${WHT}Default Environment${RST}                                      ${YLW}║${RST}")
                appendLine("${YLW}╚══════════════════════════════════════════════════════════╝${RST}")
                appendLine()
                if (current != null) {
                    appendLine("  Current default: ${CYN}${current.id}${RST}")
                    appendLine("  To change: ${DIM}env default <id>${RST}")
                } else {
                    appendLine("  ${DIM}No default environment set.${RST}")
                    appendLine("  To set: ${DIM}env default <id>${RST}")
                }
            }
        }

        val mgr = EnvironmentManager(context)
        val result = mgr.setDefault(id)
        return when (result) {
            is EnvironmentManager.ValidationResult.Success -> {
                "${GRN}Default environment set to: ${result.entry.id}${RST}"
            }
            is EnvironmentManager.ValidationResult.NotFound -> {
                "${RED}Error:${RST} Environment '${id}' not found.\n" +
                    "Use ${CYN}env list${RST} to see available environments."
            }
            else -> {
                "${RED}Error:${RST} Could not set default to '${id}'."
            }
        }
    }

    fun showInfo(context: Context): String {
        val mgr = EnvironmentManager(context)
        val envs = mgr.list()

        if (envs.isEmpty()) {
            return buildString {
                appendLine("${YLW}╔══════════════════════════════════════════════════════════╗${RST}")
                appendLine("${YLW}║${RST}  ${WHT}Environment Information${RST}                                  ${YLW}║${RST}")
                appendLine("${YLW}╚══════════════════════════════════════════════════════════╝${RST}")
                appendLine()
                appendLine("${DIM}No environments installed.${RST}")
                appendLine("Use ${CYN}env create${RST} to register one.")
            }
        }

        return buildString {
            for ((i, entry) in envs.withIndex()) {
                val running = mgr.isRunning(entry.id)
                val dir = entry.rootfsDir(context)

                appendLine("${YLW}╔══════════════════════════════════════════════════════════╗${RST}")
                appendLine("${YLW}║${RST}  ${WHT}${entry.name}${RST}                                           ${YLW}║${RST}")
                appendLine("${YLW}╚══════════════════════════════════════════════════════════╝${RST}")
                appendLine()
                appendLine("  ${WHT}ID:${RST}         ${CYN}${entry.id}${RST}")
                appendLine("  ${WHT}Distro:${RST}     ${entry.distro}")
                appendLine("  ${WHT}Arch:${RST}       ${entry.arch}")
                appendLine("  ${WHT}Shell:${RST}      ${entry.shell}")
                appendLine("  ${WHT}Status:${RST}     ${if (running) "${GRN}Running${RST}" else "${DIM}Stopped${RST}"}")
                appendLine("  ${WHT}Path:${RST}       ${dir.absolutePath}")
                appendLine("  ${WHT}Created:${RST}    ${entry.createdAt}")
                if (entry.notes.isNotEmpty()) {
                    appendLine("  ${WHT}Notes:${RST}     ${entry.notes}")
                }

                val exists = dir.exists()
                val fileCount = if (exists) dir.walkTopDown().count() - 1 else 0
                appendLine()
                appendLine("  ${WHT}Rootfs:${RST}     ${if (exists) "${GRN}${fileCount} files${RST}" else "${RED}not extracted${RST}"}")
                appendLine()
            }
        }
    }

    fun showSettings(context: Context): String {
        val settings = TeraxSettings(context)
        val data = settings.load()
        val mgr = EnvironmentManager(context)

        return buildString {
            appendLine("${YLW}╔══════════════════════════════════════════════════════════╗${RST}")
            appendLine("${YLW}║${RST}  ${WHT}Terax Global Configuration${RST}                               ${YLW}║${RST}")
            appendLine("${YLW}╚══════════════════════════════════════════════════════════╝${RST}")
            appendLine()
            appendLine("  ${WHT}Settings file:${RST} ${DIM}${settings.filePath()}${RST}")
            appendLine()
            appendLine("  ${WHT}Version:${RST}              ${data.version}")
            appendLine("  ${WHT}Default environment:${RST}  ${data.defaultEnvironmentId?.let { "${CYN}${it}${RST}" } ?: "${DIM}(none)${RST}"}")
            appendLine("  ${WHT}Boot timeout:${RST}         ${data.bootTimeoutSeconds}s")
            appendLine("  ${WHT}Fallback shell:${RST}       ${data.fallbackShell}")
            appendLine("  ${WHT}Environments:${RST}         ${data.installedEnvironments.size} registered")
            appendLine()
            appendLine("  ${WHT}Active now:${RST}           ${mgr.activeEnvironmentIds().let { ids -> if (ids.isEmpty()) "${DIM}(none)${RST}" else ids.joinToString(", ") { "${CYN}${it}${RST}" } }}")
        }
    }

    fun createEnvironment(context: Context, args: List<String>): String {
        if (args.isEmpty()) {
            return "${RED}Error:${RST} Missing environment ID.\n" +
                "Usage: env create <id> [--name <name>] [--distro <distro>] [--shell <shell>] [--arch <arch>]"
        }

        val id = args[0]
        var name = id
        var distro = "unknown"
        var shell = "/bin/sh"
        var arch = ProotEnvironment(context).hostArch
        var notes = ""

        var i = 1
        while (i < args.size) {
            when (args[i]) {
                "--name", "-n" -> if (i + 1 < args.size) { name = args[i + 1]; i += 2 }
                "--distro", "-d" -> if (i + 1 < args.size) { distro = args[i + 1]; i += 2 }
                "--shell", "-s" -> if (i + 1 < args.size) { shell = args[i + 1]; i += 2 }
                "--arch", "-a" -> if (i + 1 < args.size) { arch = args[i + 1]; i += 2 }
                "--notes" -> if (i + 1 < args.size) { notes = args[i + 1]; i += 2 }
                else -> return "${RED}Error:${RST} Unknown option: ${args[i]}"
            }
        }

        val mgr = EnvironmentManager(context)
        val result = mgr.register(id, name, distro, shell, arch, notes)

        return when (result) {
            is EnvironmentManager.ValidationResult.Success -> {
                buildString {
                    appendLine("${YLW}╔══════════════════════════════════════════════════════════╗${RST}")
                    appendLine("${YLW}║${RST}  ${WHT}Environment Created${RST}                                       ${YLW}║${RST}")
                    appendLine("${YLW}╚══════════════════════════════════════════════════════════╝${RST}")
                    appendLine()
                    appendLine("${GRN}Registered environment: ${result.entry.id}${RST}")
                    appendLine()
                    appendLine("  ${WHT}ID:${RST}         ${CYN}${result.entry.id}${RST}")
                    appendLine("  ${WHT}Name:${RST}       ${result.entry.name}")
                    appendLine("  ${WHT}Distro:${RST}     ${result.entry.distro}")
                    appendLine("  ${WHT}Arch:${RST}       ${result.entry.arch}")
                    appendLine("  ${WHT}Shell:${RST}      ${result.entry.shell}")
                    appendLine("  ${WHT}Path:${RST}       ${result.entry.rootfsDir(context).absolutePath}")
                    appendLine()
                    appendLine("Next steps:")
                    appendLine("  1. Extract a rootfs to the path above")
                    appendLine("  2. Set as default: ${DIM}env default ${result.entry.id}${RST}")
                    appendLine("  3. Launch: ${DIM}proot ${result.entry.id} /bin/sh${RST}")
                }
            }
            is EnvironmentManager.ValidationResult.DuplicateId -> {
                "${RED}Error:${RST} Environment ID '${id}' already exists.\n" +
                    "Use a different ID or delete the existing one first:\n" +
                    "  ${DIM}env delete ${id}${RST}"
            }
            is EnvironmentManager.ValidationResult.InvalidId -> {
                "${RED}Error:${RST} Invalid environment ID: ${result.reason}"
            }
            else -> {
                "${RED}Error:${RST} Could not create environment '${id}'."
            }
        }
    }
}
