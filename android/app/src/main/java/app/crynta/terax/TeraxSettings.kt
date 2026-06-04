package app.crynta.terax

import android.content.Context
import java.io.File
import org.json.JSONArray
import org.json.JSONObject

class TeraxSettings(private val context: Context) {

    data class EnvironmentEntry(
        val id: String,
        val name: String,
        val distro: String,
        val arch: String,
        val shell: String,
        val path: String,
        val createdAt: String,
        val notes: String = "",
    ) {
        fun toJson(): JSONObject = JSONObject().apply {
            put("id", id)
            put("name", name)
            put("distro", distro)
            put("arch", arch)
            put("shell", shell)
            put("path", path)
            put("created_at", createdAt)
            put("notes", notes)
        }

        fun rootfsDir(context: Context): File = File(context.filesDir, path)

        companion object {
            fun fromJson(json: JSONObject): EnvironmentEntry = EnvironmentEntry(
                id = json.getString("id"),
                name = json.optString("name", json.getString("id")),
                distro = json.optString("distro", "unknown"),
                arch = json.optString("arch", "aarch64"),
                shell = json.optString("shell", "/bin/sh"),
                path = json.optString("path", "environments/${json.getString("id")}"),
                createdAt = json.optString("created_at", ""),
                notes = json.optString("notes", ""),
            )
        }
    }

    data class SettingsData(
        val version: Int = 1,
        val defaultEnvironmentId: String? = null,
        val bootTimeoutSeconds: Int = 30,
        val fallbackShell: String = "/bin/sh",
        val installedEnvironments: List<EnvironmentEntry> = emptyList(),
    ) {
        fun toJson(): JSONObject = JSONObject().apply {
            put("version", version)
            put("default_environment_id", defaultEnvironmentId ?: JSONObject.NULL)
            put("boot_timeout_seconds", bootTimeoutSeconds)
            put("fallback_shell", fallbackShell)
            put("installed_environments", JSONArray().apply {
                installedEnvironments.forEach { put(it.toJson()) }
            })
        }

        companion object {
            fun fromJson(json: JSONObject): SettingsData = SettingsData(
                version = json.optInt("version", 1),
                defaultEnvironmentId = json.optString("default_environment_id", null)
                    ?.takeIf { it != "null" && it.isNotEmpty() },
                bootTimeoutSeconds = json.optInt("boot_timeout_seconds", 30),
                fallbackShell = json.optString("fallback_shell", "/bin/sh"),
                installedEnvironments = json.optJSONArray("installed_environments")
                    ?.let { arr ->
                        (0 until arr.length()).map { i ->
                            EnvironmentEntry.fromJson(arr.getJSONObject(i))
                        }
                    } ?: emptyList(),
            )
        }
    }

    private val settingsFile: File
        get() = File(context.filesDir, "terax_settings.json")

    @Synchronized
    fun load(): SettingsData {
        if (!settingsFile.exists()) {
            return SettingsData()
        }
        return try {
            val text = settingsFile.readText()
            SettingsData.fromJson(JSONObject(text))
        } catch (e: Exception) {
            SettingsData()
        }
    }

    @Synchronized
    fun save(data: SettingsData) {
        val tmp = File(settingsFile.absolutePath + ".tmp")
        try {
            tmp.writeText(data.toJson().toString(2))
            tmp.renameTo(settingsFile)
        } catch (e: Exception) {
            tmp.delete()
            throw e
        }
    }

    fun exists(): Boolean = settingsFile.exists()

    fun filePath(): String = settingsFile.absolutePath
}
