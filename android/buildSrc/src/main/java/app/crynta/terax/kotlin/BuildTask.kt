import java.io.File
import org.apache.tools.ant.taskdefs.condition.Os
import org.gradle.api.DefaultTask
import org.gradle.api.GradleException
import org.gradle.api.logging.LogLevel
import org.gradle.api.tasks.Input
import org.gradle.api.tasks.TaskAction

open class BuildTask : DefaultTask() {
    @Input
    var rootDirRel: String? = null
    @Input
    var target: String? = null
    @Input
    var release: Boolean? = null

    @TaskAction
    fun assemble() {
        val target = target ?: throw GradleException("target cannot be null")
        val release = release ?: throw GradleException("release cannot be null")

        // Skip if native library already up-to-date
        val targetAbi = when (target) {
            "aarch64" -> "arm64-v8a"
            "armv7" -> "armeabi-v7a"
            "i686" -> "x86"
            "x86_64" -> "x86_64"
            else -> null
        }
        if (targetAbi != null) {
            val libFile = File(project.projectDir, "src/main/jniLibs/$targetAbi/libterax_lib.so")
            val cargoLock = File(project.projectDir, "../../src-tauri/Cargo.lock")
            if (libFile.exists() && (!cargoLock.exists() || libFile.lastModified() >= cargoLock.lastModified())) {
                project.logger.lifecycle("Native library for $target is up-to-date, skipping build.")
                return
            }
        }

        val executable = project.findProperty("pnpmPath") as? String
            ?: project.rootProject.findProperty("pnpmPath") as? String
            ?: "pnpm"
        try {
            runTauriCli(executable)
        } catch (e: Exception) {
            if (Os.isFamily(Os.FAMILY_WINDOWS)) {
                // Try different Windows-specific extensions
                val fallbacks = listOf(
                    "$executable.exe",
                    "$executable.cmd",
                    "$executable.bat",
                )
                
                var lastException: Exception = e
                for (fallback in fallbacks) {
                    try {
                        runTauriCli(fallback)
                        return
                    } catch (fallbackException: Exception) {
                        lastException = fallbackException
                    }
                }
                throw lastException
            } else {
                throw e;
            }
        }
    }

    fun runTauriCli(executable: String) {
        val rootDirRel = rootDirRel ?: throw GradleException("rootDirRel cannot be null")
        val target = target ?: throw GradleException("target cannot be null")
        val release = release ?: throw GradleException("release cannot be null")

        // Build args: use `tauri android build` instead of `android-studio-script`
        val args = mutableListOf("tauri", "android", "build")
        args.add("--target")
        args.add(target)
        if (release) {
            args.add("--release")
        }
        // Build APK (will generate .so files in jniLibs)
        args.add("--apk")
        // CI mode to skip prompts
        args.add("--ci")

        val pathEnv = System.getenv("PATH")
        project.exec {
            workingDir(File(project.projectDir, rootDirRel))
            executable(executable)
            args(args)
            environment("PATH", "/Users/lennardmertins/.cargo/bin:$pathEnv")
            if (project.logger.isEnabled(LogLevel.DEBUG)) {
                args("-vv")
            } else if (project.logger.isEnabled(LogLevel.INFO)) {
                args("-v")
            }
        }.assertNormalExitValue()
    }
}