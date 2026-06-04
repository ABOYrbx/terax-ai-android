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
const BASHRC_FILENAME: &str = ".bashrc";

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

# Onboarding: check if any proot rootfs is installed on first interactive
# shell.  Prints a welcome banner only once (.terax_welcomed sentinel).
if [ -n "$PS1" ] && [ ! -f "$HOME/.terax_welcomed" ]; then
  touch "$HOME/.terax_welcomed"
  _found=0
  for _d in alpine ubuntu debian archlinux; do
    [ -d "${PREFIX}/var/rootfs/${_d}/bin" ] && _found=1
  done
  if [ "$_found" -eq 0 ]; then
    printf '\033[1;36m╔══════════════════════════════════════════════════════════════╗\033[0m\n'
    printf '\033[1;36m║\033[0m  \033[1;37mWelcome to Terax on Android\033[0m                             \033[1;36m║\033[0m\n'
    printf '\033[1;36m║\033[0m                                                          \033[1;36m║\033[0m\n'
    printf '\033[1;36m║\033[0m  To get started, run:                                     \033[1;36m║\033[0m\n'
    printf '\033[1;36m║\033[0m                                                          \033[1;36m║\033[0m\n'
    printf '\033[1;36m║\033[0m    \033[1;32msetup-distro\033[0m    Choose a Linux distribution           \033[1;36m║\033[0m\n'
    printf '\033[1;36m║\033[0m    \033[1;32mtermux-setup\033[0m    Install Termux package manager        \033[1;36m║\033[0m\n'
    printf '\033[1;36m║\033[0m                                                          \033[1;36m║\033[0m\n'
    printf '\033[1;36m╚══════════════════════════════════════════════════════════════════╝\033[0m\n'
    printf '\n'
  fi
  unset _found _d
fi

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
# backup/restore cycles, or OEM "optimisations".  Fix directory search (+x)
# across every subdirectory so the shell can traverse the tree, then make all
# files in bin/, libexec/, opt/ executable.  Without the recursive directory
# pass, newly created subdirs (package installs, helper trees) block execve()
# with EACCES even when the command file itself has +x.
#
# The parent of $PREFIX (the app's base/ dir) can lose search (+x) first,
# which makes find "$PREFIX" fail silently because the OS can't traverse
# into it.  Fix it before recursing into $PREFIX.
chmod +x "$(dirname "$PREFIX")" 2>/dev/null || true
find "$PREFIX" -type d -exec chmod +x {} + 2>/dev/null || true
find "$PREFIX/bin" "$PREFIX/libexec" "$PREFIX/opt" -type f -exec chmod +x {} + 2>/dev/null || true
find "$PREFIX/lib" -type f ! -name '*.so*' -exec chmod +x {} + 2>/dev/null || true
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

# Android filesystem may strip execute bits from extracted executables.
# Without this, every command in $PREFIX/bin returns EACCES immediately
# after bootstrap installation. Fix permissions before continuing.
[ -d "$PREFIX/bin" ]      && chmod -R +x "$PREFIX/bin"     2>/dev/null || true
[ -d "$PREFIX/libexec" ]  && chmod -R +x "$PREFIX/libexec" 2>/dev/null || true

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
  "$PREFIX/bin/apt" "$@"
  rc=$?
  # Android filesystem may strip +x from extracted executables.
  # Fix permissions on bin/ and libexec/ after every apt invocation.
  [ -d "$PREFIX/bin" ]      && chmod -R +x "$PREFIX/bin"     2>/dev/null
  [ -d "$PREFIX/libexec" ]  && chmod -R +x "$PREFIX/libexec" 2>/dev/null
  return $rc
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

/// `.bashrc` is sourced by interactive bash (non-login). It sources the
/// shared `.shrc` so the permission-fix loop, PATH, and aliases apply
/// to interactive PTY sessions too — without this, bash does NOT source
/// `$ENV` or `$BASH_ENV`, and the .shrc safety net is never reached.
const BASHRC_BODY: &str = r#"# Terax Android bash init. Sources .shrc for env + permission repair.
[ -f "$HOME/.shrc" ] && . "$HOME/.shrc"
"#;

/// `setup-distro` — interactive terminal UI for selecting and installing
/// a Linux distribution for proot-based execution. Self-contained shell
/// script that renders a Terax-styled ANSI menu, downloads the rootfs
/// tarball via wget/curl on the fly, and extracts it with tar. No Kotlin
/// or app Context required — runs in any terminal session.
///
/// Placed in `$PREFIX/bin/` alongside `termux-setup` and `pkg`. User runs
/// it from the terminal when they want to install a proot distro.
const SETUP_DISTRO_SCRIPT: &str = r#"#!/system/bin/sh
# Terax: Interactive Linux Distribution Installer for proot
#
# Renders an ANSI-styled menu in the terminal, downloads a rootfs tarball
# and extracts it into $PREFIX/var/rootfs/<distro>/.
#
# Requirements: wget (or curl), tar, and ~500 MB free in $PREFIX.
# Supported architectures: aarch64, arm, x86_64, i686.
#

set -e

# ── ANSI helpers ──────────────────────────────────────────────────────────
BOLD='\033[1m'
DIM='\033[2m'
CYAN='\033[1;36m'
GREEN='\033[1;32m'
YELLOW='\033[1;33m'
RED='\033[1;31m'
WHITE='\033[1;37m'
BLUE='\033[1;34m'
GREY='\033[0;90m'
NC='\033[0m'  # No Color

bar()  { printf "${CYAN}%s${NC}\n" "$1"; }
info() { printf "${GREY}%s${NC}\n" "$1"; }
step() { printf "${CYAN}%s${NC}\n" "$1"; }

# ── Architecture detection ────────────────────────────────────────────────
detect_arch() {
  ARCH=$(uname -m 2>/dev/null || echo "aarch64")
  case "$ARCH" in
    aarch64|arm64)        ARCH_SUFFIX="aarch64" ;;
    armv7l|armv8l|arm)    ARCH_SUFFIX="arm"     ;;
    x86_64|amd64)         ARCH_SUFFIX="x86_64"  ;;
    i686|i586|i486)       ARCH_SUFFIX="i686"    ;;
    *)                    ARCH_SUFFIX="aarch64"  ;;
  esac
  printf "%s" "$ARCH_SUFFIX"
}

AS=$(detect_arch)

# ── Distribution catalog ──────────────────────────────────────────────────
distro_name() {
  case "$1" in
    alpine)    printf "Alpine Linux"    ;;
    ubuntu)    printf "Ubuntu Base"     ;;
    debian)    printf "Debian"          ;;
    archlinux) printf "Arch Linux"      ;;
    *)         printf "Unknown"         ;;
  esac
}

distro_desc() {
  case "$1" in
    alpine)    printf "Lightweight musl/busybox based, ~5 MB rootfs"       ;;
    ubuntu)    printf "Full LTS with apt/deb ecosystem, ~300 MB"           ;;
    debian)    printf "Stable and universal, apt based, ~200 MB"           ;;
    archlinux) printf "Rolling-release, pacman based, ~250 MB"           ;;
    *)         printf "" ;;
  esac
}

distro_url() {
  case "$1" in
    alpine)    printf "https://dl-cdn.alpinelinux.org/alpine/v3.21/releases/$AS/alpine-minirootfs-3.21.3-%s.tar.gz" "$AS" ;;
    ubuntu)    printf "https://cloud-images.ubuntu.com/releases/24.04/release/ubuntu-24.04-server-cloudimg-%s-root.tar.xz" "$AS" ;;
    debian)    printf "https://github.com/termux/proot-distro/releases/download/v4.0.0/debian-%s.tar.xz" "$AS" ;;
    archlinux) printf "https://github.com/termux/proot-distro/releases/download/v4.0.0/archlinux-%s.tar.xz" "$AS" ;;
    *)         printf "" ;;
  esac
}

distro_pkg() {
  case "$1" in
    alpine)    printf "apk"     ;;
    ubuntu)    printf "apt"     ;;
    debian)    printf "apt"     ;;
    archlinux) printf "pacman"  ;;
    *)         printf ""        ;;
  esac
}

distro_dir() {
  case "$1" in
    alpine)    printf "alpine"    ;;
    ubuntu)    printf "ubuntu"    ;;
    debian)    printf "debian"    ;;
    archlinux) printf "archlinux" ;;
    *)         printf ""          ;;
  esac
}

DISTRO_IDS="alpine ubuntu debian archlinux"
DISTRO_COUNT=4

# ── Banner ────────────────────────────────────────────────────────────────
show_banner() {
  printf "\033[2J\033[H"
  bar "╔══════════════════════════════════════════════════════════════════════╗"
  printf "${CYAN}║${NC} ${WHITE}TERAX${NC} ${CYAN}│${NC}  ${WHITE}BEFORE YOU INSTALL${NC}  ${GREY}[STEP 1/3]${NC}${CYAN}            ║${NC}\n"
  printf "${CYAN}║${NC} "
  info "Choose your Linux distribution for proot-based execution on Android"
  printf "  ${CYAN}║${NC}\n"
  bar "╚══════════════════════════════════════════════════════════════════════╝"
}

# ── Selection menu ────────────────────────────────────────────────────────
show_menu() {
  printf "\n"
  printf "  ${WHITE}Available distributions${NC}  ${GREY}(type number + Enter, or 0 to cancel)${NC}\n"
  printf "\n"

  n=1
  for id in $DISTRO_IDS; do
    NAME=$(distro_name "$id")
    DESC=$(distro_desc "$id")
    PKG=$(distro_pkg "$id")

    printf "  ${CYAN}[${NC}${WHITE}%s${NC}${CYAN}]${NC}  ${WHITE}%s${NC}  ${CYAN}│${NC}  ${GREY}%s${NC}\n" "$n" "$NAME" "$DESC"
    printf "      ${GREY}pkg: %s    arch: %s${NC}\n" "$PKG" "$AS"
    if [ "$n" -lt "$DISTRO_COUNT" ]; then
      printf "      ${GREY}%s${NC}\n" "$(printf '─%.0s' $(seq 1 58))"
    fi
    n=$((n + 1))
  done

  printf "\n"
  printf "  ${CYAN}>${NC} ${GREEN}Selection${NC} ${CYAN}[1-%s]${NC}: " "$DISTRO_COUNT"
}

show_invalid() {
  printf "${RED}Invalid selection.${NC} Press Enter to retry..."
  read -r _
}

# ── Download + extract ────────────────────────────────────────────────────
install_distro() {
  DISTRO_ID="$1"
  NAME=$(distro_name "$DISTRO_ID")
  URL=$(distro_url "$DISTRO_ID")
  DIR=$(distro_dir "$DISTRO_ID")
  PKG=$(distro_pkg "$DISTRO_ID")
  ROOTFS_DIR="${PREFIX}/var/rootfs/${DIR}"

  printf "\033[2J\033[H"
  bar "╔══════════════════════════════════════════════════════════════════════╗"
  printf "${CYAN}║${NC} ${WHITE}INSTALLING:${NC} ${WHITE}%s${NC}" "$NAME"
  # Pad to align the right border
  _len=$(printf "%s" "$NAME" | wc -c)
  _pad=$((45 - _len))
  printf "%${_pad}s${CYAN}║${NC}\n" ""
  printf "${CYAN}║${NC} "
  info "$(printf '%.66s' "$URL")"
  printf "${CYAN}║${NC}\n"
  bar "╚══════════════════════════════════════════════════════════════════════╝"
  printf "\n"

  mkdir -p "$ROOTFS_DIR"

  # Determine download tool
  DL_CMD=""
  DL_PROGRESS=""
  if command -v curl >/dev/null 2>&1; then
    DL_CMD="curl -L --progress-bar"
    DL_PROGRESS="curl"
  elif command -v wget >/dev/null 2>&1; then
    DL_CMD="wget -O -"
    DL_PROGRESS="wget"
  else
    printf "\n${RED}Error: Neither curl nor wget found.${NC}\n"
    printf "${GREY}Install one via: apt install curl${NC}\n"
    return 1
  fi

  printf "  ${CYAN}▸${NC} ${WHITE}Downloading rootfs...${NC}\n"
  printf "\n"

  # Download and pipe directly to tar for extraction
  TAR_FLAGS=""
  case "$URL" in
    *.tar.gz|*.tgz)  TAR_FLAGS="-xzf" ;;
    *.tar.xz)        TAR_FLAGS="-xJf" ;;
    *.tar.bz2)       TAR_FLAGS="-xjf" ;;
    *.tar)           TAR_FLAGS="-xf"  ;;
  esac

  if [ "$DL_PROGRESS" = "curl" ]; then
    (cd "$ROOTFS_DIR" && curl -L --progress-bar "$URL" | tar $TAR_FLAGS - 2>&1) || {
      rc=$?
      printf "\n${RED}Download or extraction failed (exit code: %s).${NC}\n" "$rc"
      return 1
    }
  else
    (cd "$ROOTFS_DIR" && wget -O - "$URL" 2>&1 | tar $TAR_FLAGS - 2>&1) || {
      rc=$?
      printf "\n${RED}Download or extraction failed (exit code: %s).${NC}\n" "$rc"
      return 1
    }
  fi

  # Fix permissions on the extracted rootfs
  chmod +x "$(dirname "$ROOTFS_DIR")" 2>/dev/null || true
  find "$ROOTFS_DIR" -type d -exec chmod +x {} + 2>/dev/null || true
  find "$ROOTFS_DIR" -type f -exec chmod +x {} + 2>/dev/null || true

  # Write a start script
  cat > "$ROOTFS_DIR/start.sh" << 'STARTEOF'
#!/system/bin/sh
# Terax: Enter the proot environment
exec proot -0 -r "$(dirname "$0")" -w / /bin/sh -l
STARTEOF
  chmod +x "$ROOTFS_DIR/start.sh"

  printf "\n"
  printf "  ${CYAN}▸${NC} ${WHITE}Installation complete.${NC}\n"
  printf "\n"
  bar "╔══════════════════════════════════════════════════════════════════════╗"
  printf "${CYAN}║${NC} ${GREEN}✔  DONE${NC}  ${CYAN}│${NC}  ${WHITE}%s is ready${NC}${CYAN}               ║${NC}\n" "$NAME"
  bar "╚══════════════════════════════════════════════════════════════════════╝"
  printf "\n"
  printf "  ${WHITE}Installation summary${NC}\n"
  printf "  ${GREY}%s${NC}\n" "$(printf '─%.0s' $(seq 1 40))"
  printf "  ${CYAN}Distribution:${NC}  %s\n" "$NAME"
  printf "  ${CYAN}Location:${NC}      ${GREY}%s${NC}\n" "$ROOTFS_DIR"
  printf "  ${CYAN}Package mgr:${NC}    %s\n" "$PKG"
  printf "  ${CYAN}Architecture:${NC}   %s\n" "$AS"
  printf "\n"
  printf "  ${GREY}Enter the environment:${NC}\n"
  printf "\n"
  printf "    ${CYAN}proot -0 -r %s -w / /bin/sh -l${NC}\n" "$ROOTFS_DIR"
  printf "\n"
  printf "  ${GREY}Or run:${NC}  ${CYAN}%s/start.sh${NC}\n" "$ROOTFS_DIR"
  printf "\n"
}

# ── Main ──────────────────────────────────────────────────────────────────
main() {
  show_banner

  # Check for required tools
  MISSING=""
  command -v tar >/dev/null 2>&1 || MISSING="$MISSING tar"
  if ! command -v curl >/dev/null 2>&1 && ! command -v wget >/dev/null 2>&1; then
    MISSING="$MISSING (wget or curl)"
  fi
  if [ -n "$MISSING" ]; then
    printf "\n${RED}Missing required tools:%s${NC}\n" "$MISSING"
    printf "${GREY}Install them via: pkg install%s${NC}\n" "$MISSING"
    return 1
  fi

  while :; do
    show_menu
    read -r CHOICE
    case "$CHOICE" in
      0|q|Q) printf "\n${YELLOW}Cancelled.${NC}\n"; return 0 ;;
      1|2|3|4)
        n=0
        for id in $DISTRO_IDS; do
          n=$((n + 1))
          if [ "$n" -eq "$CHOICE" ]; then
            install_distro "$id"
            return $?
          fi
        done
        ;;
      *)
        printf "\n"
        show_invalid
        printf "\n"
        ;;
    esac
  done
}

main "$@"
"#;

/// `.profile` is sourced by login shells (and `bash --login`). Use it for
/// one-time setup so we don't redo work on every PTY.
const PROFILE_BODY: &str = r#"# Terax Android login profile. Sourced by bash/zsh -l, not by sh.
# Anything expensive goes here; keep .shrc lean.

# Source .shrc for environment, aliases, and permission repair.
[ -f "$HOME/.shrc" ] && . "$HOME/.shrc"

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
    let rootfs_dir = prefix.join("var").join("rootfs");

    for dir in [&base, &home, &prefix, &prefix_bin, &tmp, &rootfs_dir] {
        fs::create_dir_all(dir).map_err(|e| format!("create_dir_all({}): {e}", dir.display()))?;
        ensure_dir_mode_755(dir);
    }

    write_if_changed(&home.join(SHRC_FILENAME), SHRC_BODY)
        .map_err(|e| format!("write {}/{}: {e}", home.display(), SHRC_FILENAME))?;
    write_if_changed(&home.join(PROFILE_FILENAME), PROFILE_BODY)
        .map_err(|e| format!("write {}/{}: {e}", home.display(), PROFILE_FILENAME))?;
    write_if_changed(&home.join(BASHRC_FILENAME), BASHRC_BODY)
        .map_err(|e| format!("write {}/{}: {e}", home.display(), BASHRC_FILENAME))?;

    // Write the `termux-setup` helper script into $PREFIX/bin so users
    // can run it from the terminal.
    let termux_setup = prefix.join(BIN_DIR_NAME).join("termux-setup");
    write_executable(&termux_setup, TERMUX_SETUP_SCRIPT)
        .map_err(|e| format!("write {}/bin/termux-setup: {e}", prefix.display()))?;

    // Write the `pkg` command (Termux-compatible apt wrapper).
    let pkg = prefix.join(BIN_DIR_NAME).join("pkg");
    write_executable(&pkg, PKG_SCRIPT)
        .map_err(|e| format!("write {}/bin/pkg: {e}", prefix.display()))?;

    // Write the `setup-distro` interactive installer for proot rootfs.
    let setup_distro = prefix.join(BIN_DIR_NAME).join("setup-distro");
    write_executable(&setup_distro, SETUP_DISTRO_SCRIPT)
        .map_err(|e| format!("write {}/bin/setup-distro: {e}", prefix.display()))?;

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

/// Ensure a directory has at least owner-read/write/search (0o755).
/// `fs::create_dir_all` honours the process umask, which on Android may
/// strip the search bit from newly created directories — leaving the shell
/// unable to traverse into `$PREFIX/bin/` to find commands.
fn ensure_dir_mode_755(dir: &Path) {
    use std::os::unix::fs::PermissionsExt;
    if let Ok(meta) = dir.metadata() {
        let perms = meta.permissions();
        let mode = perms.mode();
        let needed = 0o755;
        if mode & needed != needed {
            let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(mode | needed));
        }
    }
}

fn write_if_changed(path: &Path, content: &str) -> std::io::Result<()> {
    if let Ok(existing) = fs::read_to_string(path) {
        if existing == content {
            return Ok(());
        }
    }
    fs::write(path, content)
}

/// Write a text script to `path` with owner-only rwx (0o700). Uses
/// `OpenOptionsExt::mode` at creation time, then always calls
/// `set_permissions` explicitly — the mode argument is only honoured when the
/// OS creates a new inode; if the file already exists it is silently ignored.
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
        .mode(0o700)
        .open(path)?;
    f.write_all(content.as_bytes())?;
    f.sync_all()?;

    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;

    Ok(())
}

/// Extract a bundled binary (from e.g. `include_bytes!`) into `$PREFIX/bin/`.
///
/// On the first call the bytes are written to disk with owner-only rwx (0o700)
/// so the Linux kernel accepts `execve()` on the file.  On subsequent calls
/// the content is compared; if it matches and the file is already executable
/// the write is skipped (fast no-op).
///
/// Usage from parent module:
///
/// ```ignore
/// const MY_TOOL: &[u8] = include_bytes!("../../res/bin/my_tool");
/// extract_bundled_binary(prefix(), "my_tool", MY_TOOL)?;
/// ```
pub fn extract_bundled_binary(prefix: &Path, name: &str, data: &[u8]) -> Result<PathBuf, String> {
    use std::os::unix::fs::PermissionsExt;

    let bin_dir = prefix.join("bin");
    let target = bin_dir.join(name);

    // Fast path: already present, same bytes, and executable.
    if let Ok(existing) = fs::read(&target) {
        if existing == data {
            if let Ok(meta) = target.metadata() {
                if meta.permissions().mode() & 0o100 != 0 {
                    return Ok(target);
                }
            }
        }
    }

    fs::create_dir_all(&bin_dir).map_err(|e| format!("create {}: {e}", bin_dir.display()))?;

    fs::write(&target, data).map_err(|e| format!("write {name}: {e}"))?;

    // Explicit set_permissions is the only reliable way on Android.
    fs::set_permissions(&target, fs::Permissions::from_mode(0o700))
        .map_err(|e| format!("chmod {name}: {e}"))?;

    log::info!("extracted bundled binary: {name} -> {}", target.display());
    Ok(target)
}

/// Recursively walk `$PREFIX` and ensure executables have the owner-execute
/// bit (`0o100`) set, and that all directories have search (`+x`) permission.
/// This is the catch-all safety net for:
///
/// 1. Bootstrap entries whose zip `unix_mode()` returned `None`.
/// 2. Files that lost their sticky execute bit across app restarts.
/// 3. Packages that install helpers outside `bin/` (e.g. `libexec/`, `lib/`).
/// 4. Directories that lost their search bit, preventing the shell from
///    traversing into `$PREFIX/bin/` to find commands ("Permission denied"
///    to the folder).
///
/// Strategy is per-directory:
/// - `bin/` — everything should be executable; no heuristics needed.
/// - `libexec/`, `lib/` — only shebang scripts and ELF binaries get the bit
///   (libraries are not executables).
/// - Every directory under `$PREFIX` is ensured to have `+x` (search)
///   permission so the shell can traverse the tree.
///
/// Called on every app startup from `ensure_layout` and after bootstrap
/// extraction from `termux_pkg::install_inner`.
pub fn fix_prefix_executables(prefix: &Path) {
    // Fix the parent of prefix (the app's base/ dir) too — if it loses search
    // (+x), even fix_directory_search_permissions will silently fail because
    // metadata() on prefix requires traversing through the parent.
    if let Some(parent) = prefix.parent() {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = parent.metadata() {
            let perms = meta.permissions();
            if perms.mode() & 0o111 == 0 {
                let _ = std::fs::set_permissions(
                    parent,
                    std::fs::Permissions::from_mode(perms.mode() | 0o111),
                );
            }
        }
    }

    // First, ensure every directory under prefix has search permission.
    // Without +x on directories, the shell cannot traverse into $PREFIX/bin/
    // and returns EACCES ("Permission denied") for every command.
    fix_directory_search_permissions(prefix);

    let bin_dir = prefix.join("bin");
    if bin_dir.is_dir() {
        set_all_executable_recursive(&bin_dir);
    }

    // Many packages install helper binaries in libexec/ and lib/ alongside
    // regular shared objects; use detection there so we don't chmod .so files.
    for sub in &["libexec", "lib"] {
        let dir = prefix.join(sub);
        if dir.is_dir() {
            fix_executables_recursive(&dir);
        }
    }

    // Sideloaded packages (openjdk, nodejs, etc.) install real executables
    // under opt/; symlinks in bin/ point to them.  Without +x on the targets
    // the shell sees EACCES even though the symlink itself is fine.
    let opt_dir = prefix.join("opt");
    if opt_dir.is_dir() {
        set_all_executable_recursive(&opt_dir);
    }
}

/// Recursively walk `prefix` and ensure every directory (including prefix
/// itself) has the owner-search bit (`0o100`) set.  Without this, the shell
/// sees EACCES when trying to traverse into `$PREFIX/bin/` or any
/// subdirectory to resolve commands.  Uses `file_type` (not `is_dir`) so
/// symlinks to external directories are not followed.
fn fix_directory_search_permissions(prefix: &Path) {
    use std::os::unix::fs::PermissionsExt;
    // Fix the current directory first so read_dir on children can succeed.
    if let Ok(meta) = prefix.metadata() {
        let perms = meta.permissions();
        if perms.mode() & 0o111 == 0 {
            let _ = std::fs::set_permissions(
                prefix,
                std::fs::Permissions::from_mode(perms.mode() | 0o111),
            );
        }
    }
    let Ok(entries) = fs::read_dir(prefix) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(ftype) = entry.file_type() else {
            continue;
        };
        if !ftype.is_dir() {
            continue;
        }
        let path = entry.path();
        fix_directory_search_permissions(&path);
    }
}

/// Make every regular file under `dir` owner-executable, no questions asked.
/// Used for `bin/` where non-executables should not be present.
/// Uses `file_type` (not `is_dir`/`is_file`) so symlinks to directories
/// outside `$PREFIX` are not followed.
fn set_all_executable_recursive(dir: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(ftype) = entry.file_type() else {
            continue;
        };
        let path = entry.path();
        if ftype.is_dir() {
            set_all_executable_recursive(&path);
        } else if ftype.is_file() || ftype.is_symlink() {
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
