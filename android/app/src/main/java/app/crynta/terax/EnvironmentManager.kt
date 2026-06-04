package app.crynta.terax

import android.content.Context
import java.io.File
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.CopyOnWriteArraySet

class EnvironmentManager(private val context: Context) {

    private val settings by lazy { TeraxSettings(context) }

    private val activeProcesses = ConcurrentHashMap<String, CopyOnWriteArraySet<Process>>()

    sealed class ValidationResult {
        data class Success(val entry: TeraxSettings.EnvironmentEntry) : ValidationResult()
        data class DuplicateId(val id: String) : ValidationResult()
        data class NotFound(val id: String) : ValidationResult()
        data class StillRunning(val id: String, val processCount: Int) : ValidationResult()
        data class InvalidId(val id: String, val reason: String) : ValidationResult()
    }

    val environmentsBase: File
        get() = File(context.filesDir, "environments")

    fun environmentDir(id: String): File = File(environmentsBase, id)

    fun register(
        id: String,
        name: String,
        distro: String,
        shell: String = "/bin/sh",
        arch: String = ProotEnvironment(context).hostArch,
        notes: String = "",
    ): ValidationResult {
        val validId = validateNewId(id)
        if (validId !is ValidationResult.Success) return validId

        val data = settings.load()
        val now = java.text.SimpleDateFormat("yyyy-MM-dd'T'HH:mm:ss'Z'", java.util.Locale.US)
            .format(java.util.Date())

        val entry = TeraxSettings.EnvironmentEntry(
            id = id,
            name = name,
            distro = distro,
            arch = arch,
            shell = shell,
            path = "environments/$id",
            createdAt = now,
            notes = notes,
        )

        val updated = data.copy(
            installedEnvironments = data.installedEnvironments + entry,
        )
        settings.save(updated)

        val dir = environmentDir(id)
        dir.mkdirs()
        TermuxFileUtils.ensureDirMode(dir)

        return ValidationResult.Success(entry)
    }

    fun unregister(id: String): ValidationResult {
        val data = settings.load()
        val entry = data.installedEnvironments.find { it.id == id }
            ?: return ValidationResult.NotFound(id)

        val running = activeProcesses[id]
        if (running != null && running.isNotEmpty()) {
            return ValidationResult.StillRunning(id, running.size)
        }

        val updated = data.copy(
            installedEnvironments = data.installedEnvironments.filter { it.id != id },
            defaultEnvironmentId = if (data.defaultEnvironmentId == id) null else data.defaultEnvironmentId,
        )
        settings.save(updated)
        activeProcesses.remove(id)
        return ValidationResult.Success(entry)
    }

    fun delete(id: String): ValidationResult {
        val result = unregister(id)
        if (result !is ValidationResult.Success) return result

        val dir = environmentDir(id)
        if (dir.exists()) {
            dir.deleteRecursively()
        }
        return result
    }

    fun list(): List<TeraxSettings.EnvironmentEntry> {
        return settings.load().installedEnvironments
    }

    fun get(id: String): TeraxSettings.EnvironmentEntry? {
        return settings.load().installedEnvironments.find { it.id == id }
    }

    fun getDefault(): TeraxSettings.EnvironmentEntry? {
        val data = settings.load()
        val defaultId = data.defaultEnvironmentId ?: return list().firstOrNull()
        return get(defaultId) ?: list().firstOrNull()
    }

    fun setDefault(id: String): ValidationResult {
        val entry = get(id)
            ?: return ValidationResult.NotFound(id)

        val data = settings.load()
        settings.save(data.copy(defaultEnvironmentId = id))
        return ValidationResult.Success(entry)
    }

    fun markRunning(id: String, process: Process) {
        activeProcesses.computeIfAbsent(id) { CopyOnWriteArraySet() }.add(process)
    }

    fun markStopped(id: String, process: Process) {
        activeProcesses[id]?.remove(process)
        if (activeProcesses[id]?.isEmpty() == true) {
            activeProcesses.remove(id)
        }
    }

    fun isRunning(id: String): Boolean {
        val processes = activeProcesses[id] ?: return false
        processes.removeIf { !it.isAlive }
        return processes.isNotEmpty()
    }

    fun activeEnvironmentIds(): Set<String> = activeProcesses.keys.toSet()

    fun ensureLayout() {
        environmentsBase.mkdirs()
        TermuxFileUtils.ensureDirMode(environmentsBase)
    }

    fun settingsFile(): String = settings.filePath()

    fun bootTarget(): TeraxSettings.EnvironmentEntry? {
        val data = settings.load()
        if (data.defaultEnvironmentId != null) {
            val entry = get(data.defaultEnvironmentId)
            if (entry != null) return entry
        }
        return list().firstOrNull()
    }

    fun bootTimeoutSeconds(): Int = settings.load().bootTimeoutSeconds

    fun fallbackShell(): String = settings.load().fallbackShell

    private fun validateNewId(id: String): ValidationResult {
        if (id.isBlank()) {
            return ValidationResult.InvalidId(id, "Environment ID must not be blank")
        }
        if (!id.matches(Regex("^[a-zA-Z0-9][a-zA-Z0-9_.-]*$"))) {
            return ValidationResult.InvalidId(id,
                "Environment ID must start with a letter or digit and contain only " +
                    "letters, digits, underscores, hyphens, and dots")
        }
        if (id.length > 64) {
            return ValidationResult.InvalidId(id, "Environment ID must be 64 characters or fewer")
        }

        val existing = get(id)
        if (existing != null) {
            return ValidationResult.DuplicateId(id)
        }

        val data = settings.load()
        return ValidationResult.Success(
            TeraxSettings.EnvironmentEntry(
                id = id,
                name = id,
                distro = "unknown",
                arch = ProotEnvironment(context).hostArch,
                shell = data.fallbackShell,
                path = "environments/$id",
                createdAt = "",
            )
        )
    }
}
