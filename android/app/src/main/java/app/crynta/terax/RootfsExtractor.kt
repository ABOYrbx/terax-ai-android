package app.crynta.terax

import android.content.Context
import android.system.Os
import java.io.Closeable
import java.io.File
import java.io.FileOutputStream
import java.io.InputStream
import java.security.DigestInputStream
import java.security.MessageDigest
import java.util.zip.GZIPInputStream
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

object RootfsExtractor {

    fun interface ProgressListener {
        fun onProgress(currentName: String, filesExtracted: Int, bytesExtracted: Long)
    }

    data class Result(
        val success: Boolean,
        val filesExtracted: Int,
        val bytesExtracted: Long,
        val checksumHex: String,
        val errorMessage: String = "",
    )

    suspend fun extract(
        context: Context,
        assetPath: String,
        destDir: File,
        expectedChecksum: String = "",
        progress: ProgressListener? = null,
    ): Result = withContext(Dispatchers.IO) {
        var checksumHex = ""
        if (expectedChecksum.isNotEmpty()) {
            checksumHex = verifyAssetChecksum(context, assetPath, expectedChecksum)
        }

        var filesExtracted = 0
        var bytesExtracted = 0L

        context.assets.open(assetPath).use { assetStream ->
            val decompressed = when {
                assetPath.endsWith(".tar.gz") || assetPath.endsWith(".tgz") ->
                    GZIPInputStream(assetStream, 64 * 1024)
                assetPath.endsWith(".tar.xz") ->
                    org.tukaani.xz.XZInputStream(assetStream)
                else -> throw IllegalArgumentException("Unsupported archive: $assetPath")
            }

            TarInputStream(decompressed).use { tar ->
                var entry = tar.nextEntry()
                while (entry != null) {
                    val target = File(destDir, entry.name)

                    when (entry.type) {
                        TarEntry.TYPE_DIR -> {
                            target.mkdirs()
                            applyMode(target, entry.mode)
                        }
                        TarEntry.TYPE_SYMLINK -> {
                            target.parentFile?.mkdirs()
                            target.delete()
                            safeSymlink(entry.linkName, target.absolutePath)
                        }
                        else -> {
                            target.parentFile?.mkdirs()
                            FileOutputStream(target).use { out ->
                                val buf = ByteArray(32 * 1024)
                                var read: Int
                                while (tar.read(buf).also { read = it } != -1) {
                                    out.write(buf, 0, read)
                                }
                            }
                            applyMode(target, entry.mode)
                            filesExtracted++
                        }
                    }

                    bytesExtracted += entry.size
                    progress?.onProgress(entry.name, filesExtracted, bytesExtracted)
                    entry = tar.nextEntry()
                }
            }
        }

        Result(
            success = true,
            filesExtracted = filesExtracted,
            bytesExtracted = bytesExtracted,
            checksumHex = checksumHex,
        )
    }

    suspend fun verifyIntegrity(
        context: Context,
        assetPath: String,
        algorithm: String = "SHA-256",
    ): String = withContext(Dispatchers.IO) {
        computeChecksum(context, assetPath, algorithm)
    }

    private fun computeChecksum(
        context: Context,
        assetPath: String,
        algorithm: String,
    ): String {
        val md = MessageDigest.getInstance(algorithm)
        context.assets.open(assetPath).use { input ->
            DigestInputStream(input, md).use { dis ->
                val buf = ByteArray(32 * 1024)
                while (dis.read(buf) != -1) { }
            }
        }
        return md.digest().joinToString("") { "%02x".format(it.toInt() and 0xFF) }
    }

    private fun verifyAssetChecksum(
        context: Context,
        assetPath: String,
        expectedHex: String,
    ): String {
        val actualHex = computeChecksum(context, assetPath, "SHA-256")
        require(actualHex == expectedHex) {
            "SHA-256 mismatch for $assetPath: expected $expectedHex, got $actualHex"
        }
        return actualHex
    }

    private fun applyMode(file: File, mode: Int) {
        val permBits = mode and 0xFFF
        if (permBits != 0) {
            try {
                Os.chmod(file.absolutePath, permBits)
            } catch (_: Exception) {
                if (permBits and 0x111 != 0) {
                    file.setExecutable(true, false)
                }
            }
        }
    }

    private fun safeSymlink(target: String, link: String) {
        try {
            Os.symlink(target, link)
        } catch (_: Exception) {
            try {
                java.nio.file.Files.createSymbolicLink(
                    java.nio.file.Paths.get(link),
                    java.nio.file.Paths.get(target),
                )
            } catch (_: Exception) { }
        }
    }
}

internal data class TarEntry(
    val name: String,
    val mode: Int,
    val type: Char,
    val linkName: String,
    val size: Long,
) {
    companion object {
        const val TYPE_FILE = '0'
        const val TYPE_DIR = '5'
        const val TYPE_SYMLINK = '2'
        const val TYPE_HARDLINK = '1'
        const val TYPE_LONG_NAME = 'L'
        const val TYPE_LONG_LINK = 'K'
    }
}

internal class TarInputStream(private val input: InputStream) : Closeable {

    private var currentEntrySize: Long = 0
    private var remainingBytes: Long = 0
    private var pendingName: String? = null
    private var pendingLinkName: String? = null

    fun nextEntry(): TarEntry? {
        skipToNextHeader()

        val header = ByteArray(512)
        readFully(header)

        if (header.all { it == 0.toByte() }) {
            val second = ByteArray(512)
            readFullyOrEof(second)
            return null
        }

        val rawName = parseString(header, 0, 100)
        val rawMode = parseOctal(header, 100, 8)
        val rawSize = parseOctal(header, 124, 12)
        val rawType = header[156].toInt().toChar()
        val rawLink = parseString(header, 157, 100)
        val prefix = parseString(header, 345, 155)

        val effectiveName = pendingName ?: if (prefix.isNotEmpty()) "$prefix/$rawName" else rawName
        val effectiveLink = pendingLinkName ?: rawLink
        pendingName = null
        pendingLinkName = null

        if (rawType == TarEntry.TYPE_LONG_NAME) {
            pendingName = readEntryText(rawSize)
            return nextEntry()
        }
        if (rawType == TarEntry.TYPE_LONG_LINK) {
            pendingLinkName = readEntryText(rawSize)
            return nextEntry()
        }

        val normalizedType = if (rawType == '\u0000') TarEntry.TYPE_FILE else rawType
        currentEntrySize = rawSize
        remainingBytes = rawSize

        return TarEntry(
            name = effectiveName,
            mode = rawMode.toInt(),
            type = normalizedType,
            linkName = effectiveLink,
            size = rawSize,
        )
    }

    fun read(buffer: ByteArray, offset: Int = 0, length: Int = buffer.size): Int {
        if (remainingBytes <= 0) return -1
        val toRead = minOf(length.toLong(), remainingBytes).toInt()
        val bytesRead = input.read(buffer, offset, toRead)
        if (bytesRead > 0) remainingBytes -= bytesRead
        return bytesRead
    }

    override fun close() {
        input.close()
    }

    private fun skipToNextHeader() {
        if (currentEntrySize > 0) {
            val consumed = currentEntrySize - remainingBytes
            val totalBlock = ((currentEntrySize + 511) / 512) * 512
            val toSkip = totalBlock - consumed
            if (toSkip > 0) skipFully(toSkip)
        }
        currentEntrySize = 0
        remainingBytes = 0
    }

    private fun readEntryText(size: Long): String {
        val data = ByteArray(size.toInt())
        readFully(data)
        val padding = (512 - (size % 512)) % 512
        if (padding > 0) skipFully(padding)
        return data.takeWhile { it != 0.toByte() }.toByteArray().decodeToString()
    }

    private fun readFully(buffer: ByteArray) {
        var offset = 0
        while (offset < buffer.size) {
            val n = input.read(buffer, offset, buffer.size - offset)
            if (n == -1) throw java.io.EOFException("Unexpected EOF in tar stream")
            offset += n
        }
    }

    private fun readFullyOrEof(buffer: ByteArray) {
        var offset = 0
        while (offset < buffer.size) {
            val n = input.read(buffer, offset, buffer.size - offset)
            if (n == -1) break
            offset += n
        }
    }

    private fun skipFully(count: Long) {
        var remaining = count
        while (remaining > 0) {
            val skipped = input.skip(remaining)
            if (skipped <= 0) {
                val buf = ByteArray(minOf(remaining, 8192L).toInt())
                val n = input.read(buf)
                if (n <= 0) throw java.io.EOFException("Unexpected EOF while skipping tar padding")
                remaining -= n
            } else {
                remaining -= skipped
            }
        }
    }

    companion object {
        const val TYPE_DIR = TarEntry.TYPE_DIR
        const val TYPE_SYMLINK = TarEntry.TYPE_SYMLINK

        private fun parseString(data: ByteArray, offset: Int, length: Int): String {
            val end = (offset until offset + length)
                .firstOrNull { data[it] == 0.toByte() }
                ?: (offset + length)
            return data.sliceArray(offset until end).decodeToString()
        }

        private fun parseOctal(data: ByteArray, offset: Int, length: Int): Long {
            val str = parseString(data, offset, length).trim()
            if (str.isEmpty()) return 0L
            return str.toLong(8)
        }
    }
}
