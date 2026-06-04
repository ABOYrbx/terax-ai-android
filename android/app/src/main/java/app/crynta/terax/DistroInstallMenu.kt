package app.crynta.terax

import android.content.Context
import java.io.BufferedInputStream
import java.io.BufferedReader
import java.io.File
import java.io.FileOutputStream
import java.io.InputStream
import java.io.InputStreamReader
import java.io.OutputStream
import java.io.PrintWriter
import java.net.HttpURLConnection
import java.net.URL
import java.util.zip.GZIPInputStream

/**
 * Terax-styled interactive terminal UI for selecting and installing
 * a Linux distribution for proot-based execution on Android.
 *
 * Renders ANSI escape sequences to [terminalOutput] and reads keyboard
 * input from [terminalInput].  Designed to be called from within a
 * [TerminalSession] pipeline: the session passes the child process's
 * stdout as [terminalOutput] and stdin as [terminalInput], so ANSI codes
 * flow to xterm.js and keystrokes flow back.
 *
 * Example (inside TerminalSession):
 * ```
 * if (needsSetup()) {
 *     val menu = DistroInstallMenu(
 *         context,
 *         output = process.outputStream,
 *         input  = process.inputStream,
 *     )
 *     menu.show()
 *     // Now start the real shell
 * }
 * ```
 *
 * Architecture:
 * - Paints a high-contrast ANSI banner with box-drawing characters
 * - Shows a numbered distribution list (keyboard-selectable)
 * - Downloads the rootfs tarball with a live progress spinner
 * - Stream-extracts via [TarInputStream] (no temp file for tar.gz)
 * - Calls [onComplete] when done, letting the caller spin up a proot session
 *
 * When called without explicit streams (default), uses System.out/in —
 * suitable for standalone invocation or child-process mode.
 */
class DistroInstallMenu(
    private val context: Context,
    private val terminalOutput: OutputStream? = null,
    private val terminalInput: InputStream? = null,
    private val onComplete: (DistroInfo) -> Unit = {},
    private val onCancel: () -> Unit = {},
    private val onError: (String) -> Unit = {},
) {

    // ── Distro catalog ──────────────────────────────────────────────────

    data class DistroInfo(
        val id: String,
        val name: String,
        val description: String,
        val rootfsUrl: String,
        val checksumUrl: String,
        val directoryName: String,
        val packageManager: String,
        val minSize: String,
    )

    private val arch: String by lazy { archName() }
    private val distros: List<DistroInfo> by lazy { catalog() }

    // ── Public entry point ──────────────────────────────────────────────

    /**
     * Run the full install flow: banner → selection → download → extract → done.
     * Blocks until completion, cancellation, or error.
     */
    fun show() {
        try {
            if (distros.isEmpty()) {
                write(red("No distributions available for architecture: $arch"))
                return
            }
            showBanner()
            val choice = showSelection()
            if (choice < 0) {
                onCancel()
                return
            }
            val distro = distros[choice]
            val destDir = File(context.filesDir, distro.directoryName)
            downloadAndExtract(distro, destDir)
            showCompletion(distro)
            onComplete(distro)
        } catch (e: InterruptedException) {
            write("\n${yellow("Interrupted.")}\n")
            onCancel()
        } catch (e: Exception) {
            write("\n${red("Error: ${e.message ?: "Unknown error"}")}\n")
            onError(e.message ?: "Unknown error")
        }
    }

    // ── Terminal I/O primitives ─────────────────────────────────────────

    private val out: PrintWriter by lazy {
        PrintWriter(terminalOutput ?: System.out, true)
    }

    private val reader: BufferedReader by lazy {
        BufferedReader(InputStreamReader(terminalInput ?: System.`in`))
    }

    private fun write(s: String) { out.print(s); out.flush() }
    private fun writeln(s: String) { out.println(s); out.flush() }
    private fun writef(fmt: String, vararg args: Any?) {
        out.format(fmt, *args); out.flush()
    }

    private fun ansi(code: String) = "\u001b[$code"
    private fun fg(code: Int) = ansi("38;5;${code}m")
    private fun bold() = ansi("1m")
    private fun dim() = ansi("2m")
    private fun reset() = ansi("0m")
    private fun clear() = ansi("2J") + ansi("H")
    private fun save() = ansi("s")
    private fun restore() = ansi("u")
    private fun up(n: Int) = ansi("${n}A")
    private fun down(n: Int) = ansi("${n}B")
    private fun forward(n: Int) = ansi("${n}C")
    private fun col(n: Int) = ansi("${n}G")

    // Terax palette (matches theme engine cyans + muted tones)
    private val CYAN = 43       // bright cyan
    private val GREEN = 76      // bright green
    private val YELLOW = 220     // bright yellow
    private val RED = 196       // bright red
    private val WHITE = 231     // bright white
    private val GREY = 59       // muted grey-green
    private val DARK = 235      // near-black background
    private val BLUE = 39       // bright blue for accents

    private fun cyan(s: String) = "${fg(CYAN)}$s${reset()}"
    private fun green(s: String) = "${fg(GREEN)}$s${reset()}"
    private fun yellow(s: String) = "${fg(YELLOW)}$s${reset()}"
    private fun red(s: String) = "${fg(RED)}$s${reset()}"
    private fun white(s: String) = "${bold()}${fg(WHITE)}$s${reset()}"
    private fun grey(s: String) = "${fg(GREY)}$s${reset()}"
    private fun blue(s: String) = "${fg(BLUE)}$s${reset()}"
    private fun highlighted(s: String) = "${bold()}${fg(CYAN)}$s${reset()}"

    // ── Banner ──────────────────────────────────────────────────────────

    private fun showBanner() {
        write(clear())
        val w = "╔══════════════════════════════════════════════════════════════════════╗"
        write(cyan("$w\n"))
        write(cyan("║"))
        write(white("  TERAX  "))      // brand
        write(cyan("│  "))
        write(white("BEFORE YOU INSTALL"))
        write(cyan("  "))
        write(grey("[STEP 1/3]"))
        write(cyan("            ║\n"))
        write(cyan("║"))
        write(grey("  Choose your Linux distribution for proot-based execution on Android"))
        write(cyan("  ║\n"))
        write(cyan("╚══════════════════════════════════════════════════════════════════════╝\n"))
    }

    // ── Selection menu ───────────────────────────────────────────────────

    private fun showSelection(): Int {
        writeln("")
        writeln("  ${white("Available distributions")}  ${grey("(type number + Enter, or 0 to cancel)")}")
        writeln("")

        for ((i, d) in distros.withIndex()) {
            val num = i + 1
            write("  ${cyan("[")}")
            write(white("$num"))
            write(cyan("]  "))
            write(white(d.name))
            write("  ")
            write(cyan("│  "))
            write(grey(d.description))
            writeln("")
            write("      ")
            write(grey("pkg: ${d.packageManager}"))
            write("  ")
            write(grey("size: ${d.minSize}"))
            write("  ")
            write(grey("arch: $arch"))
            writeln("")
            if (i < distros.lastIndex) writeln("      ${grey("─".repeat(58))}")
        }

        writeln("")
        write("  ${cyan(">")} ${green("Selection")} ${cyan("[1-${distros.size}]")}: ")
        out.flush()

        val input = readLineTrimmed()
        if (input == null || input == "0" || input == "q") return -1
        val choice = input.toIntOrNull()?.minus(1)
        if (choice == null || choice !in distros.indices) {
            write(up(1) + ansi("2K") + col(1))
            write("  ${red("Invalid selection.")} Press Enter to retry...")
            readLineTrimmed()
            write(up(1) + ansi("2K") + col(1))
            write(up(1) + ansi("2K") + col(1))
            write(up(1) + ansi("2K") + col(1))
            write(up(1) + ansi("2K") + col(1))
            // Rewind the option list
            val linesPerOption = if (distros.size <= 1) 2 else 3
            val totalLines = 1 + (distros.size * linesPerOption) + 3 // header + options + prompt
            write(up(totalLines))
            return showSelection()
        }
        return choice
    }

    // ── Download + extract ──────────────────────────────────────────────

    private fun downloadAndExtract(distro: DistroInfo, destDir: File) {
        write(clear())
        writeln("")
        write(cyan("╔══════════════════════════════════════════════════════════════════════╗\n"))
        write(cyan("║"))
        write(white("  INSTALLING: ${padEnd(distro.name, 40)}"))
        write(cyan("║\n"))
        write(cyan("║"))
        write(grey("  ${distro.rootfsUrl.take(68)}"))
        write(cyan("║\n"))
        write(cyan("╚══════════════════════════════════════════════════════════════════════╝\n"))
        writeln("")

        destDir.mkdirs()
        TermuxFileUtils.ensureDirMode(destDir)

        // Step 1: Download
        writeln("  ${cyan("▸")} ${white("Downloading rootfs...")}")

        val connection = URL(distro.rootfsUrl).openConnection() as HttpURLConnection
        connection.apply {
            connectTimeout = 30_000
            readTimeout = 60_000
            setRequestProperty("User-Agent", "Terax/1.0 (Android)")
            instanceFollowRedirects = true
        }

        val totalBytes = connection.contentLengthLong
        val inputStream = BufferedInputStream(connection.inputStream, 64 * 1024)

        val spinner = Spinner()
        writeln("")
        val spinnerLine = 3  // relative position for spinner

        // Step 2: extract
        writeln("")
        write(up(1))
        writeln("  ${cyan("▸")} ${white("Extracting filesystem...")}")
        writeln("")

        // Progress line — updated in-place
        val progressLine = 6
        write("\n".repeat(2))

        try {
            val decompressed: InputStream = when {
                distro.rootfsUrl.endsWith(".tar.gz") || distro.rootfsUrl.endsWith(".tgz") ->
                    GZIPInputStream(inputStream, 64 * 1024)
                distro.rootfsUrl.endsWith(".tar.xz") -> {
                    writeln("  ${yellow("XZ compressed — downloading to temp file first")}")
                    val tmp = File(context.cacheDir, "rootfs-${distro.id}.tar.xz")
                    downloadToFile(connection, tmp, totalBytes)
                    org.tukaani.xz.XZInputStream(BufferedInputStream(tmp.inputStream(), 64 * 1024))
                }
                else -> inputStream
            }

            var count = 0
            TarInputStream(decompressed).use { tar ->
                var entry = tar.nextEntry()

                while (entry != null) {
                    val target = File(destDir, entry.name)

                    if (entry.name.contains("..")) {
                        entry = tar.nextEntry()
                        continue
                    }

                    when (entry.type) {
                        TarInputStream.TYPE_DIR -> {
                            target.mkdirs()
                            applyMode(target, entry.mode)
                        }
                        TarInputStream.TYPE_SYMLINK -> {
                            target.parentFile?.mkdirs()
                            try {
                                android.system.Os.symlink(entry.linkName, target.absolutePath)
                            } catch (_: Exception) { /* best-effort */ }
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
                            count++
                        }
                    }

                    // Live progress
                    write(save())
                    val pct = if (totalBytes > 0) {
                        "%.1f%%".format(count.toFloat() / 1000f.coerceAtLeast(1f))
                    } else "${count} files"

                    write(up(1) + ansi("2K"))
                    write(col(1))
                    write("  ${spinner.next()} ${grey(entry.name.takeLast(60).padStart(60))}  ${cyan(pct)}")
                    write(restore())

                    // Throttle redraw
                    if (count % 3 == 0) out.flush()
                    entry = tar.nextEntry()
                }
            }

            // Clear progress lines
            write(up(2) + ansi("2K") + "\n" + ansi("2K") + "\n" + ansi("2K"))
            write(col(1))
            writeln("  ${green("✔")} ${white("Installation complete — " + count + " files extracted")}")
        } catch (e: Exception) {
            write(up(2) + ansi("2K") + "\n" + ansi("2K"))
            write(col(1))
            writeln("  ${red("✘ Extraction failed: ${e.message}")}")
            throw e
        } finally {
            try { inputStream.close() } catch (_: Exception) {}
        }

        writeln("")
        write(cyan("╚${"═".repeat(70)}╝\n"))
    }

    private fun downloadToFile(
        connection: HttpURLConnection,
        target: File,
        totalBytes: Long,
    ) {
        connection.inputStream.use { input ->
            FileOutputStream(target).use { output ->
                val buf = ByteArray(64 * 1024)
                var read: Int
                var totalRead = 0L
                val spinner = Spinner()

                while (input.read(buf).also { read = it } != -1) {
                    output.write(buf, 0, read)
                    totalRead += read
                    val pct = if (totalBytes > 0) {
                        "%.1f%%".format(totalRead.toFloat() / totalBytes.toFloat() * 100f)
                    } else {
                        "%.1f MB".format(totalRead.toFloat() / 1_048_576f)
                    }
                    write(save())
                    write(up(1) + ansi("2K"))
                    write(col(1))
                    write("  ${spinner.next()} ${grey(pct)}")
                    write(restore())
                }
            }
        }
    }

    // ── Completion ──────────────────────────────────────────────────────

    private fun showCompletion(distro: DistroInfo) {
        writeln("")
        write(clear())
        writeln("")
        write(cyan("╔══════════════════════════════════════════════════════════════════════╗\n"))
        write(cyan("║"))
        write(green("  ✔  DONE"))
        write(cyan("  │  "))
        write(white("${distro.name} is ready"))
        write(cyan("               ║\n"))
        write(cyan("╚══════════════════════════════════════════════════════════════════════╝\n"))
        writeln("")
        writeln("  ${white("Installation summary")}")
        writeln("  ${grey("─".repeat(40))}")
        writeln("  ${cyan("Distribution:")}  ${distro.name}")
        writeln("  ${cyan("Location:")}      ${grey(distro.directoryName)}")
        writeln("  ${cyan("Package mgr:")}    ${distro.packageManager}")
        writeln("  ${cyan("Architecture:")}   $arch")
        writeln("")
        writeln("  ${grey("Type the following in the terminal to enter the environment:")}")
        writeln("")
        writeln("    ${cyan("proot-distro login ${distro.id}")}")
        writeln("")
        writeln("  ${grey("Or run this to start a shell directly:")}")
        writeln("")
        writeln("    ${cyan("./start-${distro.id}.sh")}")
        writeln("")
    }

    // ── Helpers ─────────────────────────────────────────────────────────

    private fun padEnd(s: String, n: Int): String =
        if (s.length >= n) s else s + " ".repeat(n - s.length)

    private fun archName(): String {
        val a = System.getProperty("os.arch") ?: "aarch64"
        return when {
            a.contains("aarch64") || a.contains("arm64") -> "aarch64"
            a.contains("armv7") || a.contains("armv8l") -> "arm"
            a.contains("x86_64") || a.contains("amd64") -> "x86_64"
            a.contains("i686") || a.contains("i586") -> "i686"
            else -> a
        }
    }

    private fun readLineTrimmed(): String? {
        return try {
            reader.readLine()?.trim()
        } catch (_: Exception) {
            null
        }
    }

    private fun applyMode(file: File, mode: Int) {
        val permBits = mode and 0xFFF
        if (permBits != 0) {
            try {
                android.system.Os.chmod(file.absolutePath, permBits)
            } catch (_: Exception) {
                if (permBits and 0x111 != 0) {
                    file.setExecutable(true, false)
                }
            }
        }
    }

    private fun catalog(): List<DistroInfo> {
        val a = arch
        val archSuffix = when (a) {
            "aarch64" -> "aarch64"
            "arm" -> "arm"
            "x86_64" -> "x86_64"
            "i686" -> "i686"
            else -> a
        }

        return listOf(
            DistroInfo(
                id = "alpine",
                name = "Alpine Linux",
                description = "Lightweight musl/busybox based, ~5 MB rootfs, security-first",
                rootfsUrl = "https://dl-cdn.alpinelinux.org/alpine/v3.21/releases/$a/alpine-minirootfs-3.21.3-$archSuffix.tar.gz",
                checksumUrl = "https://dl-cdn.alpinelinux.org/alpine/v3.21/releases/$a/alpine-minirootfs-3.21.3-$archSuffix.tar.gz.sha256",
                directoryName = "alpine",
                packageManager = "apk",
                minSize = "~5 MB",
            ),
            DistroInfo(
                id = "ubuntu",
                name = "Ubuntu Base",
                description = "Full-featured LTS with apt/deb ecosystem, large package selection",
                rootfsUrl = "https://cloud-images.ubuntu.com/releases/24.04/release/ubuntu-24.04-server-cloudimg-$archSuffix-root.tar.xz",
                checksumUrl = "https://cloud-images.ubuntu.com/releases/24.04/release/SHA256SUMS",
                directoryName = "ubuntu",
                packageManager = "apt",
                minSize = "~300 MB",
            ),
            DistroInfo(
                id = "debian",
                name = "Debian",
                description = "Stable and universal, the foundation of Ubuntu — apt based",
                rootfsUrl = "https://github.com/termux/proot-distro/releases/download/v4.0.0/debian-$archSuffix.tar.xz",
                checksumUrl = "",
                directoryName = "debian",
                packageManager = "apt",
                minSize = "~200 MB",
            ),
            DistroInfo(
                id = "archlinux",
                name = "Arch Linux",
                description = "Rolling-release, pacman-based, bleeding edge packages",
                rootfsUrl = "https://github.com/termux/proot-distro/releases/download/v4.0.0/archlinux-$archSuffix.tar.xz",
                checksumUrl = "",
                directoryName = "archlinux",
                packageManager = "pacman",
                minSize = "~250 MB",
            ),
        )
    }

    // ── Spinner ─────────────────────────────────────────────────────────

    private class Spinner {
        private val frames = listOf("⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏")
        private var i = 0
        fun next(): String = frames[i.also { i = (i + 1) % frames.size }]
    }

    // ── Entry point ─────────────────────────────────────────────────────

    companion object {
        @JvmStatic
        fun main(args: Array<String>) {
            println("\u001b[31mDistroInstallMenu requires a Context.\u001b[0m")
            println("\u001b[90mUse DistroInstallMenu(context).show() from Android code.\u001b[0m")
        }
    }
}
