package app.crynta.terax

import android.Manifest
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.os.Environment
import android.provider.Settings
import androidx.activity.enableEdgeToEdge
import androidx.activity.result.contract.ActivityResultContracts
import androidx.core.app.ActivityCompat
import androidx.core.content.ContextCompat
import androidx.lifecycle.lifecycleScope
import java.io.File
import kotlinx.coroutines.launch

class MainActivity : TauriActivity() {
  private val REQUEST_READ_EXTERNAL = 1001

  private val openDocumentLauncher =
    registerForActivityResult(ActivityResultContracts.StartActivityForResult()) { result ->
      val data = result.data
      if (data != null) {
        val uri = data.data
        if (uri != null) {
          try {
            val takeFlags = data.flags and (Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_GRANT_WRITE_URI_PERMISSION)
            contentResolver.takePersistableUriPermission(uri, takeFlags)
          } catch (_: Exception) { }
        }
      }
    }

  private val versionCode: Int by lazy {
    try {
      val pkg = packageManager.getPackageInfo(packageName, 0)
      if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) {
        pkg.longVersionCode.toInt()
      } else {
        @Suppress("DEPRECATION")
        pkg.versionCode
      }
    } catch (_: Exception) {
      0
    }
  }

  override fun onCreate(savedInstanceState: Bundle?) {
    enableEdgeToEdge()
    super.onCreate(savedInstanceState)
    ensureStorageAccess()
    initSandbox()
  }

  /**
   * Sets up the Termux-style directory layout, extracts the rootfs
   * tarball and bundled binaries from APK assets, and fixes permissions.
   *
   * On first boot (or after an app update) the rootfs is extracted
   * asynchronously on [Dispatchers.IO] to avoid blocking the main thread.
   * The WebView loads in parallel; the Rust backend polls [.terax_ready]
   * and only proceeds once Kotlin-side init finishes.
   */
  private fun initSandbox() {
    val env = TermuxEnvironment(this)

    if (ExtractionStateManager.isExtractionRequired(this, versionCode)) {
      lifecycleScope.launch {
        extractRootfs(env)
        initSandboxSync(env)
      }
    } else {
      initSandboxSync(env)
    }
  }

  private suspend fun detectAndExtractRootfs() {
    val candidates = listOf("rootfs.tar.gz", "rootfs.tar.xz", "rootfs.tgz")
    val assetPath = candidates.firstOrNull {
      try {
        assets.open(it).close()
        true
      } catch (_: Exception) {
        false
      }
    } ?: return

    ExtractionStateManager.markInProgress(this)
    try {
      val result = RootfsExtractor.extract(
        context = this,
        assetPath = assetPath,
        destDir = filesDir,
      )
      if (result.success) {
        ExtractionStateManager.markExtracted(this, versionCode, result.checksumHex)
      }
    } catch (e: Exception) {
      ExtractionStateManager.markFailed(this)
    }
  }

  private suspend fun extractRootfs(env: TermuxEnvironment) {
    detectAndExtractRootfs()
  }

  /**
   * Fast path: layout + bundled binaries + proot + permissions + marker.
   * Runs synchronously when the rootfs is already extracted or absent.
   */
  private fun initSandboxSync(env: TermuxEnvironment) {
    TermuxFileUtils.ensureDirMode(filesDir)
    var dir: File? = env.prefix.parentFile
    while (dir != null) {
      TermuxFileUtils.ensureDirMode(dir)
      if (dir == filesDir) break
      dir = dir.parentFile
    }

    env.ensureLayout()
    extractBundledBinaries(env)
    extractProotBinary(env)
    EnvironmentManager(this).ensureLayout()
    bootstrapSettings()
    TermuxFileUtils.fixPermissionsRecursive(env.prefix)
    File(filesDir, ".terax_ready").writeText("ok")
  }

  private fun extractBundledBinaries(env: TermuxEnvironment) {
    TermuxFileUtils.extractFromAssetsRecursive(
      context = this,
      assetsDir = "bin",
      destDir = env.binDir,
      makeExec = true,
      overwrite = false,
    )
  }

  private fun extractProotBinary(env: TermuxEnvironment) {
    val prootEnv = ProotEnvironment(this)
    val arch = prootEnv.hostArch
    val assetsProotDir = "proot/$arch"

    val entries = try { assets.list(assetsProotDir) } catch (_: Exception) { null }
    if (entries.isNullOrEmpty()) return

    val destName = "proot-$arch"
    val destFile = File(env.binDir, destName)

    if (destFile.exists()) return

    try {
      assets.open("$assetsProotDir/proot").use { input ->
        destFile.outputStream().use { output -> input.copyTo(output) }
      }
      TermuxFileUtils.ensureExecutable(destFile)
    } catch (_: Exception) { }
  }

  /**
   * Initialises the global settings file on first boot with safe defaults.
   * No-op if settings already exist.  Environments are registered and
   * configured through the internal CLI (env create / env default).
   */
  private fun bootstrapSettings() {
    val settings = TeraxSettings(this)
    if (settings.exists()) return
    settings.save(TeraxSettings.SettingsData())
  }

  private fun ensureStorageAccess() {
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
      if (!Environment.isExternalStorageManager()) {
        try {
          val intent = Intent(Settings.ACTION_MANAGE_APP_ALL_FILES_ACCESS_PERMISSION)
          intent.data = Uri.parse("package:$packageName")
          startActivity(intent)
        } catch (e: Exception) {
          val intent = Intent(Settings.ACTION_MANAGE_ALL_FILES_ACCESS_PERMISSION)
          startActivity(intent)
        }
      }
    } else {
      val perm = Manifest.permission.READ_EXTERNAL_STORAGE
      if (ContextCompat.checkSelfPermission(this, perm) != PackageManager.PERMISSION_GRANTED) {
        ActivityCompat.requestPermissions(this, arrayOf(perm), REQUEST_READ_EXTERNAL)
      }
    }
  }

  fun openSafPicker(mimeTypes: Array<String> = arrayOf("*/*")) {
    val intent = Intent(Intent.ACTION_OPEN_DOCUMENT).apply {
      addCategory(Intent.CATEGORY_OPENABLE)
      type = if (mimeTypes.size == 1) mimeTypes[0] else "*/*"
      putExtra(Intent.EXTRA_MIME_TYPES, mimeTypes)
      flags = Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION
    }
    openDocumentLauncher.launch(intent)
  }
}
