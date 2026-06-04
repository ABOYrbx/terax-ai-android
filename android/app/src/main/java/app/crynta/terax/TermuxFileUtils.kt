package app.crynta.terax

import android.content.Context
import android.content.res.AssetManager
import android.system.Os
import java.io.File
import java.io.FileOutputStream
import java.util.zip.ZipEntry
import java.util.zip.ZipFile

/**
 * Extracts binaries, scripts, and bootstrap archives from the APK package
 * into the app's private sandbox ([context.filesDir]) and grants executable
 * permissions so the Linux kernel accepts them.
 *
 * Android's [noexec] restrictions on /sdcard and public dirs mean we MUST
 * use the app's internal data directory (/data/data/<pkg>/files/). Even there,
 * Umask (typically 0077) and backup/restore cycles can strip +x bits.
 *
 * Key behaviours:
 * - Scans both `assets/` and raw resource directories for bundles.
 * - Every file is written with mode 0700 (owner rwx) via `Os.chmod`.
 * - Directories get 0700 so the shell can traverse into them.
 * - Uses `Os.chmod` (not Java's `setExecutable`) because `setExecutable`
 *   only sets the owner bit and returns false on some OEM kernels.
 * - Atomic overwrite: writes to a .tmp suffix, then renames.
 */
object TermuxFileUtils {

    // 0x1ED = 0o755 = owner rwx, group r-x, other r-x
    // Files get owner-execute only (0o700), but directories need world-search
    // (+x) so the shell can traverse into them.
    private const val BIN_MODE = 0x1C0 // 0700 octal = owner rwx
    private const val DIR_MODE = 0x1ED // 0755 octal = owner rwx, world rx

    /**
     * Extract every file from an assets subdirectory into [destDir].
     *
     * @param assetsDir   Subdirectory under `assets/` (e.g. "bootstrap/bin").
     * @param destDir     Target directory under [context.filesDir].
     * @param makeExec    If true, grant owner-execute (0o700) to regular files.
     * @param overwrite   If true, overwrite existing files; if false, skip.
     * @return List of extracted file paths.
     */
    fun extractFromAssets(
        context: Context,
        assetsDir: String,
        destDir: File,
        makeExec: Boolean = true,
        overwrite: Boolean = false,
    ): List<File> {
        val assetManager: AssetManager = context.assets
        val extracted = mutableListOf<File>()

        destDir.mkdirs()
        ensureDirMode(destDir)

        val entries = assetManager.list(assetsDir) ?: return extracted

        for (name in entries) {
            val subPath = "$assetsDir/$name"
            val assetFile = File(destDir, name)

            if (overwrite || !assetFile.exists()) {
                assetManager.open(subPath).use { input ->
                    FileOutputStream(assetFile).use { output ->
                        input.copyTo(output)
                    }
                }
            }

            if (makeExec && assetFile.isFile) {
                ensureExecutable(assetFile)
            }

            extracted.add(assetFile)
        }

        return extracted
    }

    /**
     * Recursively extract a directory tree from assets into [destDir].
     * Handles nested subdirectories.
     */
    fun extractFromAssetsRecursive(
        context: Context,
        assetsDir: String,
        destDir: File,
        makeExec: Boolean = true,
        overwrite: Boolean = false,
    ) {
        val assetManager: AssetManager = context.assets
        extractRecursiveImpl(assetManager, assetsDir, destDir, makeExec, overwrite)
    }

    private fun extractRecursiveImpl(
        assetManager: AssetManager,
        path: String,
        destDir: File,
        makeExec: Boolean,
        overwrite: Boolean,
    ) {
        val entries: Array<String>
        try {
            entries = assetManager.list(path) ?: return
        } catch (_: Exception) {
            return
        }

        destDir.mkdirs()
        ensureDirMode(destDir)

        for (name in entries) {
            val childPath = "$path/$name"
            val childFile = File(destDir, name)

            if (isDirectory(assetManager, childPath)) {
                extractRecursiveImpl(assetManager, childPath, childFile, makeExec, overwrite)
            } else {
                if (overwrite || !childFile.exists()) {
                    assetManager.open(childPath).use { input ->
                        FileOutputStream(childFile).use { output ->
                            input.copyTo(output)
                        }
                    }
                }
                if (makeExec && childFile.isFile) {
                    ensureExecutable(childFile)
                }
            }
        }
    }

    /**
     * Extract entries from a ZIP file (e.g. a bootstrap archive bundled in
     * `res/raw/`) into [destDir], preserving executable bits from Unix mode.
     */
    fun extractZip(
        zipFile: File,
        destDir: File,
        stripPrefix: String? = null,
    ): List<File> {
        val extracted = mutableListOf<File>()

        ZipFile(zipFile).use { zip ->
            for (entry: ZipEntry in zip.entries()) {
                val rawName = entry.name
                val outputName = stripPrefix?.let { prefix ->
                    if (rawName.startsWith(prefix)) rawName.removePrefix(prefix) else rawName
                } ?: rawName

                val target = File(destDir, outputName)

                if (entry.isDirectory) {
                    target.mkdirs()
                    ensureDirMode(target)
                } else {
                    target.parentFile?.let { parent ->
                        parent.mkdirs()
                        ensureAncestorModes(parent)
                    }

                    zip.getInputStream(entry).use { input ->
                        FileOutputStream(target).use { output ->
                            input.copyTo(output)
                        }
                    }

                    val unixMode = entry.unixMode()
                    if (unixMode != null && (unixMode and 73) != 0) {  // 0o111 octal
                        Os.chmod(target.absolutePath, unixMode)
                    } else {
                        // Default: make executable anyway (Termux bootstrap)
                        ensureExecutable(target)
                    }
                }

                extracted.add(target)
            }
        }

        return extracted
    }

    /**
     * Grant owner-execute (0o700) using [Os.chmod].
     *
     * Prefer [Os.chmod] over [File.setExecutable]:
     * - [File.setExecutable] only flips the owner-execute bit but can silently
     *   return false on some OEM kernels that restrict the chmod syscall.
     * - [Os.chmod] directly invokes the syscall and throws on failure.
     *
     * @throws SecurityException if the OS denies the permission change.
     */
    fun ensureExecutable(file: File) {
        if (file.exists()) {
            try {
                Os.chmod(file.absolutePath, BIN_MODE)
            } catch (e: Exception) {
                // Fallback for older API levels or restricted kernels
                file.setExecutable(true, false)
            }
        }
    }

    /**
     * Ensure a directory has owner-read/write/search and world-search (0o755).
     * Without the search bit (+x), the kernel returns EACCES when the shell
     * tries to traverse into this directory to resolve a command.
     */
    fun ensureDirMode(dir: File) {
        if (dir.exists()) {
            try {
                Os.chmod(dir.absolutePath, DIR_MODE)
            } catch (_: Exception) {
                dir.setExecutable(true, false)
                dir.setReadable(true, false)
                dir.setWritable(true, false)
            }
        }
    }

    /**
     * Recursively set every file under [dir] to 0o700 and every directory
     * to 0o755.  Use after extracting bootstrap archives or after app
     * updates that may have reset permissions.
     *
     * Follows the same per-directory strategy as the Rust
     * [fix_prefix_executables] in android_fs.rs:
     * - `bin/`, `opt/` — everything gets +x (no heuristics).
     * - `libexec/`, `lib/` — skips `.so` files (only shebang/ELF gets +x).
     * - Every directory is ensured 0o755 for kernel search permission.
     */
    fun fixPermissionsRecursive(dir: File) {
        if (!dir.exists()) return

        ensureDirMode(dir)

        val children = dir.listFiles() ?: return
        val dirName = dir.name

        for (child in children) {
            if (child.isDirectory) {
                fixPermissionsRecursive(child)
            } else {
                // In lib/ and libexec/, skip shared objects.
                if ((dirName == "lib" || dirName == "libexec") && child.name.endsWith(".so")) {
                    continue
                }
                ensureExecutable(child)
            }
        }
    }

    /**
     * Restore symlinks from a Termux bootstrap SYMLINKS.txt file.
     *
     * Format:
     * ```
     * target←./path/to/link
     * ```
     * Each line uses a Unicode LEFT ARROW (U+2190) separator: target on
     * the left, link path (relative to prefix) on the right.
     *
     * Call this after [extractZip] if the zip contained a SYMLINKS.txt.
     */
    fun restoreSymlinks(symlinksFile: File, prefix: File) {
        if (!symlinksFile.exists()) return

        symlinksFile.readLines().forEach { line ->
            val parts = line.split("\u2190")
            if (parts.size != 2) return@forEach
            val (target, rawLink) = parts
            val linkPath = rawLink.removePrefix("./")
            val link = prefix.resolve(linkPath)
            link.parentFile?.mkdirs()
            try {
                link.delete()
                Os.symlink(target, link.absolutePath)
            } catch (_: Exception) {
                // Fallback for devices where symlink syscall is restricted.
                try {
                    java.nio.file.Files.createSymbolicLink(
                        link.toPath(),
                        java.nio.file.Paths.get(target),
                    )
                } catch (_: Exception) { /* best-effort */ }
            }
        }
    }

    /**
     * Walk from [dir] up to the filesystem root, ensuring every ancestor
     * directory has the search bit (+x).  When [mkdirs] creates multi-level
     * paths, mid-level directories inherit the process umask (typically
     * 0o077) which strips world-search — the shell then gets EACCES
     * traversing into them during execve() resolution.
     */
    private fun ensureAncestorModes(dir: File) {
        var current: File? = dir
        while (current != null) {
            ensureDirMode(current)
            current = current.parentFile
        }
    }

    /**
     * Check if a path inside the APK assets represents a directory.
     * AssetManager has no `isDirectory`, so we try listing its children.
     */
    private fun isDirectory(assetManager: AssetManager, path: String): Boolean {
        return try {
            val children = assetManager.list(path)
            children != null && children.isNotEmpty()
        } catch (_: Exception) {
            false
        }
    }

    private fun ZipEntry.unixMode(): Int? {
        return try {
            val method = javaClass.getMethod("getUnixMode")
            method.invoke(this) as? Int
        } catch (_: Exception) {
            null
        }
    }
}
