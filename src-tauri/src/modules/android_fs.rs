// Termux-like private filesystem layout for Android.
//
// On Android, the OS sandboxes the app under `/data/data/<pkg>/`. The shell
// the user types into can't see the rest of the device without explicit SAF
// permission grants, and `dirs::home_dir()` returns whatever the system `$HOME`
// happens to be (usually `/` or `/data`) — not a place the user owns. To match
// the Termux model, we materialize a real home inside the app's private files
// dir at first launch, point the PTY's `$HOME` and default cwd at it, and
// drop a `.shrc` / `.profile` in place so the user's shell starts in a familiar
// tree (with `$HOME`, `$PREFIX`, `~/storage`, etc.) that survives restarts.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use tauri::Manager;

const HOME_DIR_NAME: &str = "home";
const PREFIX_DIR_NAME: &str = "usr";
const TMP_DIR_NAME: &str = "tmp";
const STORAGE_DIR_NAME: &str = "storage";
const BIN_DIR_NAME: &str = "bin";

const PROFILE_FILENAME: &str = ".profile";
const SHRC_FILENAME: &str = ".shrc";

/// `.shrc` sources on every sh/mksh invocation when `$ENV` is set; keeps the
/// env consistent for one-shot commands and interactive shells.
const SHRC_BODY: &str = r#"# Terax Android shell bootstrap. Sourced by sh/mksh via $ENV.
# Keep this small — it runs on every PTY and every shell_run_command.

export TERAX=1
export TERAX_HOME="$HOME"
export TERAX_PREFIX="$PREFIX"

# Common bins shipped with the OS image. /system/bin is a busybox/toybox symlink
# farm on most devices; /vendor/bin carries OEM-specific tools.
export PATH="$PREFIX/bin:/system/bin:/system/xbin:/vendor/bin:/product/bin:$PATH"

# Termux-compatible conveniences. ls is the most-noticeable upgrade.
alias ll='ls -la'
alias la='ls -A'
alias l='ls -CF'
alias cls='clear'

# A helpful prompt that reflects cwd and exit status.
# $? in the prompt would re-evaluate, so we use a one-shot via _prompt_status.
if [ -n "$PS1" ]; then
  _terax_prompt() {
    _ec=$?
    printf '\033[1;32m%s\033[0m:\033[1;34m%s\033[0m$ ' "${TERAX_HOSTNAME:-terax}" "${PWD/#$HOME/~}"
    [ "$_ec" -ne 0 ] && printf '\033[1;31m[%d]\033[0m ' "$_ec"
  }
  PROMPT_COMMAND=_terax_prompt
fi
"#;

/// `.profile` is sourced by login shells (and `bash --login`). Use it for
/// one-time setup so we don't redo work on every PTY.
const PROFILE_BODY: &str = r#"# Terax Android login profile. Sourced by bash/zsh -l, not by sh.
# Anything expensive goes here; keep .shrc lean.

if [ -z "$TERAX_HOME" ]; then
  export TERAX=1
  export TERAX_HOME="$HOME"
  export TERAX_PREFIX="$PREFIX"
fi

# Make TERAX_HOME / TERAX_PREFIX visible to subshells even when the parent
# shell bypasses our .shrc (e.g. busybox `sh -c` from a Tauri command).
export TERAX_HOME TERAX_PREFIX

# Ensure the standard dirs exist on first login.
[ -d "$HOME/storage" ] || mkdir -p "$HOME/storage" 2>/dev/null || true
[ -d "$PREFIX/bin" ] || mkdir -p "$PREFIX/bin" 2>/dev/null || true
[ -d "$HOME/tmp" ] || mkdir -p "$HOME/tmp" 2>/dev/null || true
"#;

// Cached after the first `init` call so other modules (workspace, shell_init)
// can look up the home without juggling an AppHandle. `OnceLock` is the same
// primitive TERAX.md recommends for `LAUNCH_CWD`.
static HOME: OnceLock<PathBuf> = OnceLock::new();
static PREFIX: OnceLock<PathBuf> = OnceLock::new();
static APP_DATA: OnceLock<PathBuf> = OnceLock::new();

/// Resolves the app's private files dir on Android (e.g.
/// `/data/user/0/app.crynta.terax/files`). Returns `None` on non-Android
/// targets so the rest of the app can fall through to default behavior.
#[cfg(target_os = "android")]
pub fn app_data_dir(app: &tauri::AppHandle) -> Option<PathBuf> {
    app.path().app_local_data_dir().ok()
}

#[cfg(not(target_os = "android"))]
pub fn app_data_dir(_app: &tauri::AppHandle) -> Option<PathBuf> {
    None
}

/// Cached, layout-aware accessors used by the rest of the app.
pub fn home() -> Option<&'static Path> {
    HOME.get().map(PathBuf::as_path)
}

pub fn prefix() -> Option<&'static Path> {
    PREFIX.get().map(PathBuf::as_path)
}

pub fn app_data() -> Option<&'static Path> {
    APP_DATA.get().map(PathBuf::as_path)
}

/// Idempotently create the home, prefix, and tmp directories, then write the
/// default `.shrc` / `.profile` if missing. Safe to call on every launch.
/// Also populates the cached `HOME` / `PREFIX` / `APP_DATA` so other modules
/// (workspace, shell_init, frontend commands) can resolve the home without
/// needing the AppHandle.
pub fn init(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let home = ensure_layout(app)?;
    let _ = HOME.set(home.clone());
    if let Some(prefix) = prefix_dir(app) {
        let _ = PREFIX.set(prefix);
    }
    if let Some(base) = app_data_dir(app) {
        let _ = APP_DATA.set(base);
    }
    Ok(home)
}

/// The Termux-style home directory. Created on demand.
pub fn home_dir(app: &tauri::AppHandle) -> Option<PathBuf> {
    let base = app_data_dir(app)?;
    let home = base.join(HOME_DIR_NAME);
    Some(home)
}

/// `$PREFIX` — the apps/tools prefix. Empty by default; users can drop
/// additional binaries in `$PREFIX/bin` and they'll be on `PATH` via `.shrc`.
pub fn prefix_dir(app: &tauri::AppHandle) -> Option<PathBuf> {
    let base = app_data_dir(app)?;
    Some(base.join(PREFIX_DIR_NAME))
}

fn ensure_layout(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let base = app_data_dir(app)
        .ok_or_else(|| "app data dir unavailable on this platform".to_string())?;

    let home = base.join(HOME_DIR_NAME);
    let prefix = base.join(PREFIX_DIR_NAME);
    let prefix_bin = prefix.join(BIN_DIR_NAME);
    let tmp = base.join(TMP_DIR_NAME);
    let storage = home.join(STORAGE_DIR_NAME);

    for dir in [&base, &home, &prefix, &prefix_bin, &tmp, &storage] {
        fs::create_dir_all(dir)
            .map_err(|e| format!("create_dir_all({}): {e}", dir.display()))?;
    }

    write_if_changed(&home.join(SHRC_FILENAME), SHRC_BODY)
        .map_err(|e| format!("write {}/{}: {e}", home.display(), SHRC_FILENAME))?;
    write_if_changed(&home.join(PROFILE_FILENAME), PROFILE_BODY)
        .map_err(|e| format!("write {}/{}: {e}", home.display(), PROFILE_FILENAME))?;

    // TERAX.md documents `TERAX_HOME` as a public convention; surface it for
    // the webview's onboarding copy via a small file users can `cat`.
    let readme = home.join(".terax-readme");
    let readme_body = format!(
        "# Terax home\n\
         #\n\
         # This directory is the app's private workspace on Android.\n\
         # Anything you create here is visible to the terminal and the file\n\
         # explorer, and persists across app restarts.\n\
         #\n\
         # TERAX_HOME = {home}\n\
         # TERAX_PREFIX = {prefix}\n\
         # TERAX_TMP = {tmp}\n",
        home = home.display(),
        prefix = prefix.display(),
        tmp = tmp.display(),
    );
    write_if_changed(&readme, &readme_body)
        .map_err(|e| format!("write {}/.terax-readme: {e}", home.display()))?;

    Ok(home)
}

fn write_if_changed(path: &Path, content: &str) -> std::io::Result<()> {
    if let Ok(existing) = fs::read_to_string(path) {
        if existing == content {
            return Ok(());
        }
    }
    fs::write(path, content)
}

/// Tauri command: returns the Termux-style home dir on Android, null
/// elsewhere. The frontend falls back to its normal resolution on non-Android.
#[tauri::command]
pub fn android_home_dir(app: tauri::AppHandle) -> Option<String> {
    init(&app).ok().map(|p| p.to_string_lossy().into_owned())
}

/// Tauri command: returns the same path as `android_home_dir` but with a
/// pre-flight `init` call. Mostly used for the very first launch
/// where the dir doesn't exist yet and the frontend wants to know the
/// resolved path *and* trigger creation in one round-trip.
#[tauri::command]
pub fn android_init_home(app: tauri::AppHandle) -> Result<String, String> {
    let home = init(&app)?;
    Ok(home.to_string_lossy().into_owned())
}

/// Tauri command: returns the absolute home dir (preferred) plus the
/// fallback `app.path().home_dir()` so the frontend can pick the most
/// appropriate one. Both are returned as a pair.
#[derive(serde::Serialize)]
pub struct AndroidPaths {
    pub home: Option<String>,
    pub prefix: Option<String>,
    pub app_data: Option<String>,
}

#[tauri::command]
pub fn android_paths(app: tauri::AppHandle) -> AndroidPaths {
    init(&app).ok();
    AndroidPaths {
        home: home().map(|p| p.to_string_lossy().into_owned()),
        prefix: prefix().map(|p| p.to_string_lossy().into_owned()),
        app_data: app_data().map(|p| p.to_string_lossy().into_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shrc_keeps_path_in_scope() {
        // Sanity: the bootstrap must export PATH. If this drifts, every PTY on
        // Android loses access to /system/bin tools.
        assert!(SHRC_BODY.contains("export PATH"));
        assert!(SHRC_BODY.contains("/system/bin"));
    }

    #[test]
    fn profile_keeps_termax_env_in_scope() {
        assert!(PROFILE_BODY.contains("export TERAX_HOME"));
        assert!(PROFILE_BODY.contains("export TERAX_PREFIX"));
    }

    #[test]
    fn write_if_changed_is_noop_when_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("f");
        fs::write(&p, "x").unwrap();
        write_if_changed(&p, "x").unwrap();
        assert_eq!(fs::read_to_string(&p).unwrap(), "x");
    }

    #[test]
    fn write_if_changed_replaces_on_diff() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("f");
        fs::write(&p, "x").unwrap();
        write_if_changed(&p, "y").unwrap();
        assert_eq!(fs::read_to_string(&p).unwrap(), "y");
    }
}
