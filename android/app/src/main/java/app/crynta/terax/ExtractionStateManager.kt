package app.crynta.terax

import android.content.Context
import android.content.SharedPreferences

object ExtractionStateManager {

    private const val PREFS_NAME = "terax_extraction"
    private const val KEY_VERSION = "extraction_version"
    private const val KEY_STATE = "extraction_state"
    private const val KEY_CHECKSUM = "extraction_checksum"
    private const val KEY_TIMESTAMP = "extraction_timestamp"

    enum class State {
        NOT_EXTRACTED,
        IN_PROGRESS,
        COMPLETED,
        FAILED,
    }

    fun isExtractionRequired(
        context: Context,
        currentVersionCode: Int,
        assetChecksum: String = "",
    ): Boolean {
        val prefs = prefs(context)
        val state = readState(prefs)
        val storedVersion = prefs.getInt(KEY_VERSION, -1)
        val storedChecksum = prefs.getString(KEY_CHECKSUM, "") ?: ""

        if (state == State.COMPLETED && storedVersion == currentVersionCode) {
            if (assetChecksum.isEmpty() || storedChecksum == assetChecksum) {
                return false
            }
        }
        return true
    }

    fun markInProgress(context: Context) {
        prefs(context).edit()
            .putString(KEY_STATE, State.IN_PROGRESS.name)
            .apply()
    }

    fun markExtracted(
        context: Context,
        versionCode: Int,
        checksum: String = "",
    ) {
        prefs(context).edit()
            .putInt(KEY_VERSION, versionCode)
            .putString(KEY_STATE, State.COMPLETED.name)
            .putString(KEY_CHECKSUM, checksum)
            .putLong(KEY_TIMESTAMP, System.currentTimeMillis())
            .apply()
    }

    fun markFailed(context: Context) {
        prefs(context).edit()
            .putString(KEY_STATE, State.FAILED.name)
            .apply()
    }

    fun getState(context: Context): State {
        return readState(prefs(context))
    }

    fun getExtractedVersion(context: Context): Int {
        return prefs(context).getInt(KEY_VERSION, -1)
    }

    fun reset(context: Context) {
        prefs(context).edit().clear().apply()
    }

    private fun prefs(context: Context): SharedPreferences {
        return context.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE)
    }

    private fun readState(prefs: SharedPreferences): State {
        return try {
            State.valueOf(prefs.getString(KEY_STATE, State.NOT_EXTRACTED.name)!!)
        } catch (_: Exception) {
            State.NOT_EXTRACTED
        }
    }
}
