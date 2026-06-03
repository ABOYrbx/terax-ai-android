use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use serde::Serialize;
use tauri::ipc::Channel;

/// Termux architecture names mapped from Rust cfgs.
pub fn termux_arch() -> &'static str {
    #[cfg(target_arch = "aarch64")]
    { "aarch64" }
    #[cfg(target_arch = "arm")]
    { "arm" }
    #[cfg(target_arch = "x86_64")]
    { "x86_64" }
    #[cfg(target_arch = "x86")]
    { "i686" }
    #[cfg(not(any(
        target_arch = "aarch64",
        target_arch = "arm",
        target_arch = "x86_64",
        target_arch = "x86"
    )))]
    { "unknown" }
}

/// The GitHub API URL to discover the latest bootstrap release.
const BOOTSTRAP_RELEASES_API: &str =
    "https://api.github.com/repos/termux/termux-packages/releases?per_page=10";

/// The sources.list content that makes `apt` point at the official Termux repo.
const SOURCES_LIST: &str = "\
deb https://packages.termux.dev/apt/termux-main/ stable main
# Uncomment to enable X11 support:
# deb https://packages.termux.dev/apt/termux-x11/ x11 main
# Uncomment for root packages:
# deb https://packages.termux.dev/apt/termux-root/ root stable
";

static BOOTSTRAP_INSTALLING: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, Serialize)]
pub struct BootstrapRelease {
    pub version: String,
    pub url: String,
    pub size: u64,
    pub checksum_sha256: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BootstrapStatus {
    pub installed: bool,
    pub arch: String,
    pub prefix: Option<String>,
    pub installing: bool,
}

#[derive(Debug, Clone, Serialize)]
pub enum BootstrapEvent {
    #[serde(rename = "progress")]
    Progress { message: String, percent: f64 },
    #[serde(rename = "error")]
    Error { message: String },
    #[serde(rename = "done")]
    Done,
    #[serde(rename = "log")]
    Log { message: String },
}

/// Check if the Termux bootstrap is installed by looking for `apt` in `$PREFIX/bin`.
pub fn is_installed() -> bool {
    let prefix = match crate::modules::android_fs::prefix() {
        Some(p) => p.to_path_buf(),
        None => return false,
    };
    let apt = prefix.join("bin").join("apt");
    apt.exists()
}

/// Get the bootstrap status (used by Tauri commands).
pub fn status() -> BootstrapStatus {
    let installed = is_installed();
    let prefix = crate::modules::android_fs::prefix()
        .map(|p| p.to_string_lossy().to_string());
    BootstrapStatus {
        installed,
        arch: termux_arch().to_string(),
        prefix,
        installing: BOOTSTRAP_INSTALLING.load(Ordering::Acquire),
    }
}

/// Find the latest Termux bootstrap release from GitHub.
pub async fn find_latest_bootstrap() -> Result<BootstrapRelease, String> {
    let client = reqwest::Client::builder()
        .user_agent("terax/0.1")
        .build()
        .map_err(|e| format!("reqwest client: {e}"))?;

    let resp = client
        .get(BOOTSTRAP_RELEASES_API)
        .send()
        .await
        .map_err(|e| format!("GitHub API request failed: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!("GitHub API returned {}", resp.status()));
    }

    let body = resp
        .text()
        .await
        .map_err(|e| format!("read response body: {e}"))?;
    let releases: Vec<serde_json::Value> = serde_json::from_str(&body)
        .map_err(|e| format!("parse releases JSON: {e}"))?;

    let arch = termux_arch();
    let zip_name = format!("bootstrap-{arch}.zip");

    for release in &releases {
        let tag = release["tag_name"]
            .as_str()
            .unwrap_or("");
        if !tag.contains("bootstrap-") || !tag.contains("apt.android") {
            continue;
        }
        if let Some(assets) = release["assets"].as_array() {
            for asset in assets {
                let name = asset["name"].as_str().unwrap_or("");
                if name == zip_name {
                    let url = asset["browser_download_url"]
                        .as_str()
                        .unwrap_or("")
                        .to_string();
                    let size = asset["size"].as_u64().unwrap_or(0);
                    let checksum = None; // Could parse from the release notes
                    return Ok(BootstrapRelease {
                        version: tag.trim_start_matches("bootstrap-").to_string(),
                        url,
                        size,
                        checksum_sha256: checksum,
                    });
                }
            }
        }
    }

    Err(format!(
        "no bootstrap zip found for arch '{arch}' in recent releases"
    ))
}

/// Download and install the Termux bootstrap.
/// Reports progress via the `on_event` channel.
fn emit(on_event: &Option<Channel<BootstrapEvent>>, event: BootstrapEvent) {
    if let Some(ch) = on_event {
        let _ = ch.send(event);
    } else {
        match &event {
            BootstrapEvent::Log { message }
            | BootstrapEvent::Progress { message, .. } => log::info!("[termux] {message}"),
            BootstrapEvent::Error { message } => log::error!("[termux] {message}"),
            BootstrapEvent::Done => log::info!("[termux] bootstrap install done"),
        }
    }
}

pub async fn install_bootstrap(
    app: &tauri::AppHandle,
    on_event: Channel<BootstrapEvent>,
) -> Result<(), String> {
    let ch = Some(on_event);
    emit(&ch, BootstrapEvent::Log {
        message: "Starting Termux bootstrap installation...".into(),
    });

    if is_installed() {
        emit(&ch, BootstrapEvent::Log {
            message: "Termux bootstrap is already installed.".into(),
        });
        emit(&ch, BootstrapEvent::Done);
        return Ok(());
    }

    if BOOTSTRAP_INSTALLING.swap(true, Ordering::AcqRel) {
        return Err("Bootstrap installation is already in progress".into());
    }

    let result = install_inner(app, ch.clone()).await;

    BOOTSTRAP_INSTALLING.store(false, Ordering::Release);

    if let Err(e) = &result {
        emit(&ch, BootstrapEvent::Error {
            message: e.clone(),
        });
    } else {
        emit(&ch, BootstrapEvent::Done);
    }

    result
}

async fn install_inner(
    _app: &tauri::AppHandle,
    on_event: Option<Channel<BootstrapEvent>>,
) -> Result<(), String> {
    let arch = termux_arch();
    emit(&on_event, BootstrapEvent::Log {
        message: format!("Target architecture: {arch}"),
    });

    let prefix = crate::modules::android_fs::prefix()
        .ok_or_else(|| "Android prefix dir not available".to_string())?
        .to_path_buf();

    let _ = emit(&on_event, BootstrapEvent::Progress {
        message: "Finding latest bootstrap release...".into(),
        percent: 0.0,
    });

    let release = find_latest_bootstrap().await?;

    let _ = emit(&on_event, BootstrapEvent::Progress {
        message: format!(
            "Found bootstrap release: {} ({:.1} MiB)",
            release.version,
            release.size as f64 / 1_048_576.0
        ),
        percent: 5.0,
    });

    let client = reqwest::Client::builder()
        .user_agent("terax/0.1")
        .build()
        .map_err(|e| format!("reqwest client: {e}"))?;

    // Download the bootstrap zip to a temp file
    let tmp_dir =
        tempfile::tempdir().map_err(|e| format!("create temp dir: {e}"))?;
    let zip_path = tmp_dir.path().join("bootstrap.zip");

    let _ = emit(&on_event, BootstrapEvent::Log {
        message: format!("Downloading from: {}", release.url),
    });

    let resp = client
        .get(&release.url)
        .send()
        .await
        .map_err(|e| format!("download failed: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!("download returned {}", resp.status()));
    }

    let total_size = resp.content_length().unwrap_or(release.size);
    let mut downloaded: u64 = 0;
    let mut stream = resp.bytes_stream();
    let mut file =
        std::fs::File::create(&zip_path).map_err(|e| format!("create temp file: {e}"))?;

    use futures_util::StreamExt;
    while let Some(chunk_result) = stream.next().await {
        let chunk = chunk_result.map_err(|e| format!("download chunk: {e}"))?;
        file.write_all(&chunk)
            .map_err(|e| format!("write temp file: {e}"))?;
        downloaded += chunk.len() as u64;
        let pct = 5.0 + (downloaded as f64 / total_size as f64) * 55.0;
        let _ = emit(&on_event, BootstrapEvent::Progress {
            message: format!(
                "Downloading... {:.1} MiB / {:.1} MiB",
                downloaded as f64 / 1_048_576.0,
                total_size as f64 / 1_048_576.0
            ),
            percent: pct,
        });
    }
    drop(file);

    let _ = emit(&on_event, BootstrapEvent::Progress {
        message: "Extracting bootstrap archive...".into(),
        percent: 60.0,
    });

    // Extract the zip into $PREFIX
    let zip_file = std::fs::File::open(&zip_path)
        .map_err(|e| format!("open zip: {e}"))?;
    let mut archive =
        zip::ZipArchive::new(zip_file).map_err(|e| format!("read zip archive: {e}"))?;

    let file_count = archive.len();
    let _ = emit(&on_event, BootstrapEvent::Log {
        message: format!("Archive contains {file_count} entries"),
    });

    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| format!("read zip entry {i}: {e}"))?;

        let entry_name = entry
            .name()
            .to_string();

        // Handle SYMLINKS.txt for post-process symlink creation
        if entry_name == "SYMLINKS.txt" {
            let mut content = String::new();
            entry
                .read_to_string(&mut content)
                .map_err(|e| format!("read SYMLINKS.txt: {e}"))?;
            process_symlinks(&content, &prefix)?;
            continue;
        }

        let target_path = prefix.join(&entry_name);

        if entry.is_dir() {
            let _ = std::fs::create_dir_all(&target_path);
            continue;
        }

        if let Some(parent) = target_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("create dir {}: {e}", parent.display()))?;
        }

        let mut out =
            std::fs::File::create(&target_path)
                .map_err(|e| format!("create {}: {e}", target_path.display()))?;

        std::io::copy(&mut entry, &mut out)
            .map_err(|e| format!("extract {}: {e}", target_path.display()))?;

        // Preserve executable bits from Unix mode
        if let Some(mode) = entry.unix_mode() {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&target_path, std::fs::Permissions::from_mode(mode));
        }

        let pct = 60.0 + ((i + 1) as f64 / file_count as f64) * 30.0;
        let _ = emit(&on_event, BootstrapEvent::Progress {
            message: format!(
                "Extracting... {}/{}",
                i + 1,
                file_count
            ),
            percent: pct,
        });
    }

    // Write apt sources.list
    let _ = emit(&on_event, BootstrapEvent::Log {
        message: "Configuring apt sources...".into(),
    });

    let etc_apt = prefix.join("etc").join("apt");
    std::fs::create_dir_all(&etc_apt)
        .map_err(|e| format!("create {}/etc/apt: {e}", prefix.display()))?;

    let sources_list_path = etc_apt.join("sources.list");
    write_if_changed(&sources_list_path, SOURCES_LIST)
        .map_err(|e| format!("write sources.list: {e}"))?;

    // Ensure dpkg status is correctly set up
    let dpkg_dir = prefix.join("var").join("lib").join("dpkg");
    std::fs::create_dir_all(&dpkg_dir)
        .map_err(|e| format!("create {}/var/lib/dpkg: {e}", dpkg_dir.display()))?;

    let status_path = dpkg_dir.join("status");
    if !status_path.exists() {
        std::fs::write(&status_path, "")
            .map_err(|e| format!("write dpkg status: {e}"))?;
    }

    let _ = emit(&on_event, BootstrapEvent::Progress {
        message: "Running post-install configuration...".into(),
        percent: 95.0,
    });

    // Ensure apt and dpkg are executable
    for bin in &["apt", "apt-get", "dpkg"] {
        let path = prefix.join("bin").join(bin);
        if path.exists() {
            use std::os::unix::fs::PermissionsExt;
            let meta = path.metadata().ok();
            if let Some(m) = meta {
                let perms = m.permissions();
                if perms.mode() & 0o111 == 0 {
                    let _ = std::fs::set_permissions(
                        &path,
                        std::fs::Permissions::from_mode(perms.mode() | 0o111),
                    );
                }
            }
        }
    }

    // Run `dpkg --configure -a` and `apt update` in the bootstrap context
    let _ = emit(&on_event, BootstrapEvent::Log {
        message: "Configuring packages...".into(),
    });

    // Now we need to configure: dpkg --configure -a
    // We run this via the shell command builder.
    let _ = run_apt_in_prefix(&prefix, &[
        "sh", "-c",
        &format!(
            "export PREFIX={p} HOME={h} && {p}/bin/dpkg --configure -a 2>&1 || true",
            p = prefix.display(),
            h = crate::modules::android_fs::home()
                .map(|h| h.display().to_string())
                .unwrap_or_default(),
        ),
    ]);

    let _ = emit(&on_event, BootstrapEvent::Progress {
        message: "Updating package lists...".into(),
        percent: 98.0,
    });

    // apt update
    let _ = run_apt_in_prefix(&prefix, &[
        "sh", "-c",
        &format!(
            "export PREFIX={p} HOME={h} && {p}/bin/apt-get update 2>&1 || true",
            p = prefix.display(),
            h = crate::modules::android_fs::home()
                .map(|h| h.display().to_string())
                .unwrap_or_default(),
        ),
    ]);

    let _ = emit(&on_event, BootstrapEvent::Log {
        message: "Termux bootstrap installation complete!".into(),
    });

    Ok(())
}

fn process_symlinks(content: &str, prefix: &Path) -> Result<(), String> {
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        // Format: target←link_path
        if let Some((target, link_path)) = line.split_once('←') {
            let target = target.trim();
            let link_path = link_path.trim().trim_start_matches("./");
            let full_link = prefix.join(link_path);

            if let Some(parent) = full_link.parent() {
                let _ = std::fs::create_dir_all(parent);
            }

            let _ = std::fs::remove_file(&full_link);
            if let Err(e) = std::os::unix::fs::symlink(target, &full_link) {
                log::warn!("symlink {} -> {} failed: {e}", full_link.display(), target);
            }
        }
    }
    Ok(())
}

fn run_apt_in_prefix(prefix: &Path, args: &[&str]) -> Result<String, String> {
    let output = std::process::Command::new(&args[0])
        .args(&args[1..])
        .env_clear()
        .env("PATH", format!(
            "{}/bin:/system/bin:/system/xbin:/vendor/bin",
            prefix.display()
        ))
        .env("PREFIX", prefix)
        .env("HOME", crate::modules::android_fs::home()
            .map(|h| h.to_path_buf())
            .unwrap_or_else(|| prefix.to_path_buf()))
        .env("TERM", "xterm-256color")
        .env("LD_LIBRARY_PATH", format!("{}/lib", prefix.display()))
        .output()
        .map_err(|e| format!("spawn command: {e}"))?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let combined = if stderr.is_empty() {
        stdout
    } else {
        format!("{stdout}\n{stderr}")
    };

    if !output.status.success() {
        log::warn!("apt-in-prefix exit={:?}: {combined}", output.status.code());
    }

    Ok(combined)
}

/// Run an apt command inside the Termux environment.
pub fn run_apt(args: Vec<String>) -> Result<String, String> {
    let prefix = crate::modules::android_fs::prefix()
        .ok_or_else(|| "Android prefix dir not available".to_string())?
        .to_path_buf();

    let apt = prefix.join("bin").join("apt");
    if !apt.exists() {
        return Err("Termux bootstrap is not installed. Install it first.".into());
    }

    let home_val = crate::modules::android_fs::home()
        .map(|h| h.to_string_lossy().to_string())
        .unwrap_or_default();

    let output = std::process::Command::new(&apt)
        .args(&args)
        .env_clear()
        .env("PATH", format!(
            "{}/bin:/system/bin:/system/xbin:/vendor/bin:/product/bin",
            prefix.display()
        ))
        .env("PREFIX", &prefix)
        .env("HOME", &home_val)
        .env("TERM", "xterm-256color")
        .env("LD_LIBRARY_PATH", format!("{}/lib", prefix.display()))
        .env("TMPDIR", format!("{}/tmp", prefix.display()))
        .output()
        .map_err(|e| format!("spawn apt: {e}"))?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    if output.status.success() {
        Ok(if stderr.is_empty() { stdout } else { format!("{stdout}{stderr}") })
    } else {
        // apt sometimes writes to stdout even on non-zero exit (e.g. search results)
        let combined = if stderr.is_empty() { stdout } else { format!("{stdout}\n{stderr}") };
        Err(combined)
    }
}

/// List installed packages.
pub fn list_installed() -> Result<Vec<InstalledPackage>, String> {
    let prefix = crate::modules::android_fs::prefix()
        .ok_or_else(|| "Android prefix dir not available".to_string())?
        .to_path_buf();

    let status_path = prefix.join("var").join("lib").join("dpkg").join("status");
    if !status_path.exists() {
        return Ok(Vec::new());
    }

    let content =
        std::fs::read_to_string(&status_path).map_err(|e| format!("read dpkg status: {e}"))?;

    let mut packages = Vec::new();
    let stanzas = content.split("\n\n");

    for stanza in stanzas {
        let mut name = String::new();
        let mut version = String::new();
        let mut description = String::new();
        let mut installed = false;

        for line in stanza.lines() {
            if let Some(val) = line.strip_prefix("Package: ") {
                name = val.to_string();
            } else if let Some(val) = line.strip_prefix("Version: ") {
                version = val.to_string();
            } else if let Some(val) = line.strip_prefix("Description: ") {
                description = val.to_string();
            } else if line.starts_with("Status: ") && line.contains("installed") {
                installed = true;
            }
        }

        if !name.is_empty() && installed {
            packages.push(InstalledPackage { name, version, description, installed });
        }
    }

    Ok(packages)
}

#[derive(Debug, Clone, Serialize)]
pub struct InstalledPackage {
    pub name: String,
    pub version: String,
    pub description: String,
    pub installed: bool,
}

fn write_if_changed(path: &Path, content: &str) -> std::io::Result<()> {
    if let Ok(existing) = std::fs::read_to_string(path) {
        if existing == content {
            return Ok(());
        }
    }
    std::fs::write(path, content)
}

/// Auto-install the Termux bootstrap on app startup (no progress channel).
/// Spawned as a background task from the Tauri setup hook. Logs progress
/// but is otherwise silent so the user's terminal isn't spammed.
pub async fn auto_install(app: &tauri::AppHandle) {
    if is_installed() {
        log::info!("termux bootstrap already installed, skipping auto-install");
        return;
    }

    if BOOTSTRAP_INSTALLING.swap(true, Ordering::AcqRel) {
        log::info!("termux bootstrap install already in progress");
        return;
    }

    log::info!("termux bootstrap not found, starting background install...");

    match install_inner(app, None).await {
        Ok(()) => {
            log::info!("termux bootstrap auto-install completed successfully");
        }
        Err(e) => {
            log::error!("termux bootstrap auto-install failed: {e}");
        }
    }

    BOOTSTRAP_INSTALLING.store(false, Ordering::Release);
}

// ── Tauri commands ──────────────────────────────────────────────────────

#[tauri::command]
pub fn termux_is_installed() -> bool {
    is_installed()
}

#[tauri::command]
pub fn termux_bootstrap_status() -> BootstrapStatus {
    status()
}

#[tauri::command]
pub async fn termux_install_bootstrap(
    app: tauri::AppHandle,
    on_event: Channel<BootstrapEvent>,
) -> Result<(), String> {
    install_bootstrap(&app, on_event).await
}

#[tauri::command]
pub async fn termux_run_apt(args: Vec<String>) -> Result<String, String> {
    // Uses spawn_blocking so the Tauri async runtime stays unblocked.
    tauri::async_runtime::spawn_blocking(move || run_apt(args))
        .await
        .map_err(|e| format!("apt thread panicked: {e:?}"))?
}

#[tauri::command]
pub fn termux_list_packages() -> Result<Vec<InstalledPackage>, String> {
    list_installed()
}
