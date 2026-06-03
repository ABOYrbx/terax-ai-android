// Termux-like private filesystem layout for Android.
//
// On Android, the OS sandboxes the app under `/data/data/<pkg>/`. The shell
// the user types into can't see the rest of the device without explicit SAF
// permission grants, and `dirs::home_dir()` returns whatever the system `$HOME`
// happens to be (usually `/` or `/data`) — not a place the user owns. To match
// the Termux model, we materialize a real home inside the app's private files
// dir at first launch, point the PTY's `$HOME` and default cwd at it, and
// drop a `.shrc` / `.profile` in place so the user's shell starts in a familiar
// tree (with `$HOME`, `$PREFIX`, etc.) that survives restarts.

use std::fs;
use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use tauri::Manager;

const HOME_DIR_NAME: &str = "home";
const PREFIX_DIR_NAME: &str = "usr";
const TMP_DIR_NAME: &str = "tmp";
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

# Termux binaries (apt, dpkg, ...) need LD_LIBRARY_PATH to find shared libs.
export LD_LIBRARY_PATH="$PREFIX/lib"
export TMPDIR="$PREFIX/tmp"

# Termux-compatible conveniences. ls is the most-noticeable upgrade.
alias ll='ls -la'
alias la='ls -A'
alias l='ls -CF'
alias cls='clear'

# A helpful prompt that reflects cwd and exit status.
if [ -n "$PS1" ]; then
  _terax_prompt() {
    _ec=$?
    printf '\033[1;32m%s\033[0m:\033[1;34m%s\033[0m$ ' "${TERAX_HOSTNAME:-terax}" "${PWD/#$HOME/~}"
    [ "$_ec" -ne 0 ] && printf '\033[1;31m[%d]\033[0m ' "$_ec"
  }
  PROMPT_COMMAND=_terax_prompt
fi

# Android app data dirs can lose the executable sticky bit across app restarts,
# backup/restore cycles, or OEM "optimisations".  Ensure every file in
# $PREFIX/bin has +x so pkg/apt/dpkg/clear etc. don't get EACCES on execve().
for _f in "$PREFIX"/bin/*; do
  [ -f "$_f" ] && [ ! -x "$_f" ] && chmod +x "$_f" 2>/dev/null || true
done
unset _f
"#;

/// `termux-setup` shell script placed in `$PREFIX/bin/`. Self-contained
/// installer that downloads and extracts the Termux bootstrap using system
/// tools (toybox/busybox) available on all modern Android devices.
const TERMUX_SETUP_SCRIPT: &str = r#"#!/system/bin/sh
# Terax: Termux Package Manager Setup
#
# Downloads and installs the Termux bootstrap using system tools.
# After installation, use:  apt install <package>
#
# Packages available: openssh, git, python, nodejs, build-essential, vim, ...

set -e

ARCH=$(uname -m)
case "$ARCH" in
  aarch64) BOOTSTRAP_ARCH="aarch64" ;;
  armv7l|armv8l) BOOTSTRAP_ARCH="arm" ;;
  x86_64)  BOOTSTRAP_ARCH="x86_64" ;;
  i686|i586|i486) BOOTSTRAP_ARCH="i686" ;;
  *)
    printf '\033[1;31mError:\033[0m Unsupported architecture: %s\n' "$ARCH"
    exit 1
    ;;
esac

PREFIX="${PREFIX:-/data/data/app.crynta.terax/files/usr}"

printf '\033[1;33m╔══════════════════════════════════════════════════════════╗\033[0m\n'
printf '\033[1;33m║\033[0m  \033[1;37mTerax Package Manager Setup\033[0m                                 \033[1;33m║\033[0m\n'
printf '\033[1;33m╚══════════════════════════════════════════════════════════╝\033[0m\n'
printf '\n'
printf 'Architecture: \033[1;36m%s\033[0m\n' "$BOOTSTRAP_ARCH"
printf 'Prefix:       \033[1;36m%s\033[0m\n' "$PREFIX"
printf '\n'

if [ -x "$PREFIX/bin/apt" ]; then
  printf '\033[1;32mTermux bootstrap is already installed.\033[0m\n'
  printf 'Run \033[1;36mapt update\033[0m to refresh package lists.\n'
  exit 0
fi

# Check for required tools
HAVE_CURL=0
HAVE_WGET=0
HAVE_UNZIP=0
command -v curl  >/dev/null 2>&1 && HAVE_CURL=1
command -v wget  >/dev/null 2>&1 && HAVE_WGET=1
command -v unzip >/dev/null 2>&1 && HAVE_UNZIP=1

DOWNLOAD_CMD=""
if [ "$HAVE_CURL" = "1" ]; then
  DOWNLOAD_CMD="curl -L"
elif [ "$HAVE_WGET" = "1" ]; then
  DOWNLOAD_CMD="wget -O"
fi

if [ -z "$DOWNLOAD_CMD" ] || [ "$HAVE_UNZIP" = "0" ]; then
  printf '\033[1;31mError:\033[0m Neither curl nor wget found, or unzip is missing.\n'
  printf 'Please install them from Settings -> Packages or use\n'
  printf 'a device with a full toybox/busybox installation.\n'
  exit 1
fi

TMPDIR="${TMPDIR:-/data/local/tmp}"
TMPFILE="$TMPDIR/termux-bootstrap-$$.zip"
BOOTSTRAP_URL="https://github.com/termux/termux-packages/releases/latest/download/bootstrap-${BOOTSTRAP_ARCH}.zip"

printf 'Downloading \033[1;36m%s\033[0m\n' "$BOOTSTRAP_URL"
printf 'Target:      \033[1;36m%s\033[0m\n' "$PREFIX"
printf '\n'

if [ "$HAVE_CURL" = "1" ]; then
  curl -L --progress-bar -o "$TMPFILE" "$BOOTSTRAP_URL"
else
  wget -O "$TMPFILE" "$BOOTSTRAP_URL" 2>&1
fi

printf '\n\033[1;33mExtracting bootstrap...\033[0m\n'
unzip -o "$TMPFILE" -d "$PREFIX" 2>&1 | awk 'BEGIN{ORS=" "}{print "."}END{printf "\n"}'

# Process SYMLINKS.txt if present
if [ -f "$PREFIX/SYMLINKS.txt" ]; then
  printf '\033[1;33mRestoring symlinks...\033[0m\n'
  while IFS='←' read -r target link_path; do
    [ -z "$target" ] && continue
    link_path=$(printf '%s' "$link_path" | sed 's|^\./||')
    ln -sf "$target" "$PREFIX/$link_path" 2>/dev/null || true
  done < "$PREFIX/SYMLINKS.txt"
  rm -f "$PREFIX/SYMLINKS.txt"
fi

rm -f "$TMPFILE"

# Write apt sources.list
mkdir -p "$PREFIX/etc/apt"
cat > "$PREFIX/etc/apt/sources.list" << 'EOF'
deb https://packages.termux.dev/apt/termux-main/ stable main
EOF

# Make sure dpkg status exists
mkdir -p "$PREFIX/var/lib/dpkg"
touch "$PREFIX/var/lib/dpkg/status"

printf '\n\033[1;33mConfiguring packages...\033[0m\n'
"$PREFIX/bin/dpkg" --configure -a 2>/dev/null || true

printf '\n\033[1;32m✓ Bootstrap installed successfully!\033[0m\n'
printf '\n'
printf 'Next steps:\n'
printf '  \033[1;36mapt update\033[0m         Refresh package lists\n'
printf '  \033[1;36mapt install openssh\033[0m   Install SSH client\n'
printf '  \033[1;36mapt install git\033[0m        Install Git\n'
printf '  \033[1;36mapt install python\033[0m     Install Python\n'
printf '  \033[1;36mpkg search <query>\033[0m  Search packages\n'
printf '\n'
printf 'Happy hacking! \xF0\x9F\x9A\x80\n'
"#;

/// `pkg` — Termux-compatible package manager command.
/// Wraps `apt` with the familiar `pkg install/search/remove/update/upgrade` interface.
/// Placed in $PREFIX/bin by ensure_layout so it's on PATH from the first terminal session.
const PKG_SCRIPT: &str = r#"#!/system/bin/sh
# Terax: Termux-compatible package manager (pkg -> apt wrapper)
#
# Usage:
#   pkg install     <pkg>...   Install packages
#   pkg uninstall   <pkg>...   Remove packages
#   pkg update                 Update package lists
#   pkg upgrade                Upgrade all packages
#   pkg search      <pattern>  Search for packages
#   pkg list-installed         List installed packages
#   pkg files       <pkg>      List files owned by a package
#   pkg show        <pkg>      Show package details
#   pkg reinstall   <pkg>...   Reinstall packages
#   pkg depends     <pkg>      Show dependencies of a package
#   pkg add-repo    <name>     Add an apt repository (interactive)
#   pkg remove-repo <name>     Remove an apt repository
#   pkg repo list              List configured apt repositories
#   pkg help                   Show this help

PREFIX="${PREFIX:-/data/data/app.crynta.terax/files/usr}"
SOURCES_D="$PREFIX/etc/apt/sources.list.d"

die() {
  printf '\033[1;31mError:\033[0m %s\n' "$1" >&2
  exit 1
}

warn() {
  printf '\033[1;33mWarning:\033[0m %s\n' "$1" >&2
}

help() {
  sed -n 's/^# //p; /^$/q' "$0"
  exit 0
}

require_bootstrap() {
  if [ ! -x "$PREFIX/bin/apt" ]; then
    die "Termux bootstrap is not installed.
Run '\033[1;32mtermux-setup\033[0m' first, then try again."
  fi
}

run_apt() {
  require_bootstrap
  exec "$PREFIX/bin/apt" "$@"
}

repo_list() {
  if [ ! -d "$SOURCES_D" ]; then
    printf 'No additional repositories configured.\n'
    exit 0
  fi
  found=0
  for f in "$SOURCES_D"/*.list; do
    [ -f "$f" ] || continue
    found=1
    name=$(basename "$f" .list)
    content=$(grep -v '^#' "$f" | grep -v '^$' | head -1)
    printf '\033[1;36m%s\033[0m\n' "$name"
    if [ -n "$content" ]; then
      printf '  %s\n' "$content"
    fi
  done
  [ "$found" -eq 0 ] && printf 'No additional repositories configured.\n'
}

repo_add() {
  require_bootstrap
  mkdir -p "$SOURCES_D"
  name="$1"
  [ -z "$name" ] && die "add-repo: missing repository name
Usage: pkg add-repo <name>"
  shift
  if [ $# -ge 2 ]; then
    url="$1"
    dist="$2"
    comp="$3"
    printf '%s\n' "deb $url $dist ${comp:-main}" > "$SOURCES_D/$name.list"
  else
    known=$(ls "$SOURCES_D"/ 2>/dev/null | sed 's/\.list$//')
    printf '\033[1;33mAvailable repositories:\033[0m\n'
    printf '  \033[1;36mx11\033[0m    - \033[1;37mTermux X11\033[0m    (deb https://packages.termux.dev/apt/termux-x11/ x11 main)\n'
    printf '  \033[1;36mroot\033[0m   - \033[1;37mTermux Root\033[0m   (deb https://packages.termux.dev/apt/termux-root/ root stable)\n'
    printf '  \033[1;36munstable\033[0m - \033[1;37mTermux Unstable\033[0m (deb https://packages.termux.dev/apt/termux-unstable/ unstable main)\n'
    printf '\n'
    case "$name" in
      x11)
        printf 'deb https://packages.termux.dev/apt/termux-x11/ x11 main\n' > "$SOURCES_D/$name.list"
        printf '\033[1;32mAdded repository: %s\033[0m\n' "$name"
        ;;
      root)
        printf 'deb https://packages.termux.dev/apt/termux-root/ root stable\n' > "$SOURCES_D/$name.list"
        printf '\033[1;32mAdded repository: %s\033[0m\n' "$name"
        ;;
      unstable)
        printf 'deb https://packages.termux.dev/apt/termux-unstable/ unstable main\n' > "$SOURCES_D/$name.list"
        printf '\033[1;32mAdded repository: %s\033[0m\n' "$name"
        ;;
      *)
        die "Unknown repository: $name
Known repositories: x11, root, unstable
Or use: pkg add-repo <name> <url> <distribution> [component]"
        ;;
    esac
  fi
  printf 'Run \033[1;36mpkg update\033[0m to refresh package lists.\n'
}

repo_remove() {
  require_bootstrap
  [ -z "$1" ] && die "remove-repo: missing repository name
Usage: pkg remove-repo <name>"
  file="$SOURCES_D/$1.list"
  if [ -f "$file" ]; then
    rm -f "$file"
    printf '\033[1;32mRemoved repository: %s\033[0m\n' "$1"
    printf 'Run \033[1;36mpkg update\033[0m to refresh package lists.\n'
  else
    die "Repository '$1' not found in $SOURCES_D"
  fi
}

case "${1:-help}" in
  install|add)
    shift
    [ $# -eq 0 ] && die "install: missing package name(s)
Usage: pkg install <package>..."
    run_apt install "$@"
    ;;
  uninstall|remove|rm|delete)
    shift
    [ $# -eq 0 ] && die "uninstall: missing package name(s)
Usage: pkg uninstall <package>..."
    run_apt remove "$@"
    ;;
  update)
    run_apt update
    ;;
  upgrade)
    run_apt upgrade
    ;;
  search|find)
    shift
    [ $# -eq 0 ] && die "search: missing search pattern
Usage: pkg search <pattern>"
    run_apt search "$@"
    ;;
  list-installed|list)
    require_bootstrap
    if command -v "$PREFIX/bin/dpkg" >/dev/null 2>&1; then
      exec "$PREFIX/bin/dpkg" -l
    else
      die "dpkg not found in bootstrap"
    fi
    ;;
  files|list-files)
    shift
    [ $# -eq 0 ] && die "files: missing package name
Usage: pkg files <package>"
    require_bootstrap
    exec "$PREFIX/bin/dpkg" -L "$@"
    ;;
  show|info)
    shift
    [ $# -eq 0 ] && die "show: missing package name
Usage: pkg show <package>"
    run_apt show "$@"
    ;;
  reinstall)
    shift
    [ $# -eq 0 ] && die "reinstall: missing package name(s)
Usage: pkg reinstall <package>..."
    run_apt install --reinstall "$@"
    ;;
  depends|dependencies)
    shift
    [ $# -eq 0 ] && die "depends: missing package name
Usage: pkg depends <package>"
    run_apt depends "$@"
    ;;
  add-repo)
    shift
    repo_add "$@"
    ;;
  remove-repo)
    shift
    repo_remove "$@"
    ;;
  repo)
    shift
    case "${1:-list}" in
      list) repo_list ;;
      *) die "Unknown repo subcommand: $1
Usage: pkg repo list" ;;
    esac
    ;;
  help|--help|-h)
    help
    ;;
  *)
    die "Unknown subcommand: $1
Run 'pkg help' for usage."
    ;;
esac
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
    let base =
        app_data_dir(app).ok_or_else(|| "app data dir unavailable on this platform".to_string())?;

    let home = base.join(HOME_DIR_NAME);
    let prefix = base.join(PREFIX_DIR_NAME);
    let prefix_bin = prefix.join(BIN_DIR_NAME);
    let tmp = base.join(TMP_DIR_NAME);

    for dir in [&base, &home, &prefix, &prefix_bin, &tmp] {
        fs::create_dir_all(dir).map_err(|e| format!("create_dir_all({}): {e}", dir.display()))?;
    }

    write_if_changed(&home.join(SHRC_FILENAME), SHRC_BODY)
        .map_err(|e| format!("write {}/{}: {e}", home.display(), SHRC_FILENAME))?;
    write_if_changed(&home.join(PROFILE_FILENAME), PROFILE_BODY)
        .map_err(|e| format!("write {}/{}: {e}", home.display(), PROFILE_FILENAME))?;

    // Write the `termux-setup` helper script into $PREFIX/bin so users
    // can run it from the terminal.
    let termux_setup = prefix.join(BIN_DIR_NAME).join("termux-setup");
    write_executable(&termux_setup, TERMUX_SETUP_SCRIPT)
        .map_err(|e| format!("write {}/bin/termux-setup: {e}", prefix.display()))?;

    // Write the `pkg` command (Termux-compatible apt wrapper).
    let pkg = prefix.join(BIN_DIR_NAME).join("pkg");
    write_executable(&pkg, PKG_SCRIPT)
        .map_err(|e| format!("write {}/bin/pkg: {e}", prefix.display()))?;

    // Re-apply execute permissions on every startup across the entire
    // $PREFIX tree.  Android filesystems don't always preserve the executable
    // stickiness across app restarts, backup/restore cycles, or OEM
    // "optimisations", and the bootstrap zip may not carry Unix mode metadata
    // for every entry it extracts.  Packages can also install executables
    // outside bin/ (libexec/, lib/, lib/apt/, etc.), so we do a full recursive
    // walk — not just a flat scan of bin/.
    fix_prefix_executables(&prefix);

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

/// Like `write_if_changed` but ensures the file is executable (0o755).
/// Uses `OpenOptionsExt::mode` on Unix to request execute bits at creation
/// time, then always calls `set_permissions` explicitly because `mode()` is
/// only honoured when the OS actually creates the inode — if the file already
/// exists the mode argument is silently ignored, leaving stale permissions
/// (e.g. 0o644 from a prior failed write or a backup restore) in place.
fn write_executable(path: &Path, content: &str) -> std::io::Result<()> {
    if let Ok(existing) = fs::read_to_string(path) {
        if existing == content {
            use std::os::unix::fs::PermissionsExt;
            let meta = path.metadata()?;
            let perms = meta.permissions();
            if (perms.mode() & 0o111) != 0 {
                return Ok(());
            }
        }
    }
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o755)
        .open(path)?;
    f.write_all(content.as_bytes())?;
    f.sync_all()?;

    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o755))?;

    Ok(())
}

/// Recursively walk `$PREFIX` and ensure executables have the owner-execute
/// bit (`0o100`) set.  This is the catch-all safety net for:
///
/// 1. Bootstrap entries whose zip `unix_mode()` returned `None`.
/// 2. Files that lost their sticky execute bit across app restarts.
/// 3. Packages that install helpers outside `bin/` (e.g. `libexec/`, `lib/`).
///
/// Strategy is per-directory:
/// - `bin/` — everything should be executable; no heuristics needed.
/// - `libexec/`, `lib/` — only shebang scripts and ELF binaries get the bit
///   (libraries are not executables).
///
/// Called on every app startup from `ensure_layout` and after bootstrap
/// extraction from `termux_pkg::install_inner`.
pub fn fix_prefix_executables(prefix: &Path) {
    let bin_dir = prefix.join("bin");
    if bin_dir.exists() {
        set_all_executable_recursive(&bin_dir);
    }

    // Many packages install helper binaries in libexec/ and lib/ alongside
    // regular shared objects; use detection there so we don't chmod .so files.
    for sub in &["libexec", "lib"] {
        let dir = prefix.join(sub);
        if dir.exists() {
            fix_executables_recursive(&dir);
        }
    }
}

/// Make every regular file under `dir` owner-executable, no questions asked.
/// Used for `bin/` where non-executables should not be present.
fn set_all_executable_recursive(dir: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            set_all_executable_recursive(&path);
        } else if path.is_file() || path.is_symlink() {
            if let Ok(meta) = path.metadata() {
                let perms = meta.permissions();
                if perms.mode() & 0o111 == 0 {
                    let _ = std::fs::set_permissions(
                        &path,
                        std::fs::Permissions::from_mode(perms.mode() | 0o111),
                    );
                }
            }
        }
    }
}

/// Only shebang scripts and ELF binaries get the execute bit.
fn fix_executables_recursive(dir: &Path) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            fix_executables_recursive(&path);
        } else if path.is_file() {
            // Quick gate: skip if already has some execute bit.
            use std::os::unix::fs::PermissionsExt;
            if let Ok(meta) = path.metadata() {
                let perms = meta.permissions();
                if perms.mode() & 0o111 != 0 {
                    continue;
                }
            } else {
                continue;
            }
            // Read first bytes to detect shebang or ELF magic.
            let should_exec = std::io::BufReader::new(match std::fs::File::open(&path) {
                Ok(f) => f,
                Err(_) => continue,
            })
            .fill_buf()
            .ok()
            .map(|buf| buf.starts_with(b"#!") || buf.starts_with(b"\x7fELF"))
            .unwrap_or(false);

            if should_exec {
                if let Ok(meta) = path.metadata() {
                    let perms = meta.permissions();
                    let _ = std::fs::set_permissions(
                        &path,
                        std::fs::Permissions::from_mode(perms.mode() | 0o111),
                    );
                }
            }
        }
    }
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
