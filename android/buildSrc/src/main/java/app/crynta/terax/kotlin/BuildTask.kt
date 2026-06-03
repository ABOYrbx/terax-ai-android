import java.io.File
import org.gradle.api.DefaultTask
import org.gradle.api.GradleException
import org.gradle.api.tasks.Input
import org.gradle.api.tasks.TaskAction

open class BuildTask : DefaultTask() {
    @Input
    var target: String? = null

    @TaskAction
    fun assemble() {
        val target = target ?: throw GradleException("target cannot be null")

        val targetAbi = when (target) {
            "aarch64" -> "arm64-v8a"
            "armv7" -> "armeabi-v7a"
            "i686" -> "x86"
            "x86_64" -> "x86_64"
            else -> null
        }

        if (targetAbi != null) {
            val libFile = File(project.projectDir, "src/main/jniLibs/$targetAbi/libterax_lib.so")
            if (libFile.exists()) {
                project.logger.lifecycle("Native library for $target is up-to-date, skipping build.")
                return
            }
        }

        project.logger.warn(
            "Native library for $target not found at expected location. " +
            "Run 'pnpm tauri android build' or 'pnpm tauri android dev' to build the Rust backend. " +
            "Skipping in-Gradle Rust build to avoid circular dependency with the Tauri CLI."
        )
    }
}