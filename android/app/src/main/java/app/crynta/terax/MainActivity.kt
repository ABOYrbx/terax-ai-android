package app.crynta.terax

import android.Manifest
import android.content.Intent
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.os.Environment
import android.provider.Settings
import androidx.activity.enableEdgeToEdge
import androidx.core.app.ActivityCompat
import androidx.core.content.ContextCompat
import android.content.pm.PackageManager

class MainActivity : TauriActivity() {
  private val REQUEST_READ_EXTERNAL = 1001
  private val REQUEST_OPEN_DOCUMENT = 1002

  override fun onCreate(savedInstanceState: Bundle?) {
    enableEdgeToEdge()
    super.onCreate(savedInstanceState)
    ensureStorageAccess()
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

    // Launch a Storage Access Framework picker for the user to grant access to a file or directory.
    fun openSafPicker(mimeTypes: Array<String> = arrayOf("*/*")) {
      val intent = Intent(Intent.ACTION_OPEN_DOCUMENT).apply {
        addCategory(Intent.CATEGORY_OPENABLE)
        type = if (mimeTypes.size == 1) mimeTypes[0] else "*/*"
        putExtra(Intent.EXTRA_MIME_TYPES, mimeTypes)
        flags = Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION
      }
      startActivityForResult(intent, REQUEST_OPEN_DOCUMENT)
    }

    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
      super.onActivityResult(requestCode, resultCode, data)
      if (requestCode == REQUEST_OPEN_DOCUMENT && data != null) {
        val uri = data.data
        if (uri != null) {
          try {
            val takeFlags = data.flags and (Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_GRANT_WRITE_URI_PERMISSION)
            contentResolver.takePersistableUriPermission(uri, takeFlags)
          } catch (e: Exception) {
            // ignore takePersistableUriPermission failures
          }
        }
      }
    }
}
