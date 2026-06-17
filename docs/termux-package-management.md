# Termux Package Management on Terax Android

## Overview

Terax integrates full Termux-compatible package management, allowing users to install, update, and manage Linux packages directly on Android. The system consists of two layers: a Rust backend (`termux_pkg.rs`) that handles bootstrap installation and apt execution, and a shell-based `pkg` command that provides a Termux-compatible CLI interface with additional repository management.

## Termux Bootstrap

The Termux bootstrap is a minimal root filesystem (~30 MB compressed) published by the Termux project at `github.com/termux/termux-packages`. It contains:

- **APT package manager** (`apt`, `apt-get`) for package resolution and installation
- **dpkg** for low-level package management
- **Core utilities** (`bash`, `coreutils`, `findutils`, `grep`, `sed`, etc.)
- **Essential libraries** (`libc`, `libz`, `libssl`, etc.)

### How Bootstrap Installation Works

1. **Architecture Detection**: The target architecture is mapped from Rust cfgs:
   - `aarch64` → `aarch64`
   - `armv7` → `arm`
   - `x86_64` → `x86_64`
   - `x86` → `i686`

2. **Release Discovery**: Queries the GitHub releases API (`api.github.com/repos/termux/termux-packages/releases`) to find the latest `bootstrap-{arch}.zip` release tagged with `bootstrap-*apt.android*`. Each request uses a `terax/0.1` user agent.

3. **Download**: Streams the zip archive to a temporary file, emitting progress events (0-55%) through a Tauri event channel for the Settings UI progress bar.

4. **Extraction**: Iterates over each zip entry, creating directories and writing files into `$PREFIX` (default: `/data/data/app.crynta.terax/files/usr`). Special handling exists for:
   - `SYMLINKS.txt` — A manifest file in the bootstrap archive that lists symlinks in `target←link_path` format, processed after all files are extracted. Symlinks cannot be stored in zip archives reliably on Android.
   - Unix permissions — When the zip entry provides `unix_mode()`, execute bits are preserved. When it returns `None` (common on Android), files lack execute bits — this is why the permission fixer runs immediately after extraction.

5. **Post-Install Configuration**:
   - Writes `$PREFIX/etc/apt/sources.list` pointing to `deb https://packages.termux.dev/apt/termux-main/ stable main`
   - Creates `$PREFIX/var/lib/dpkg/status` if absent
   - Calls `fix_prefix_executables()` to ensure every bin/libexec entry has the execute bit
   - Runs `dpkg --configure -a` to complete any pending package configuration
   - Runs `apt-get update` to refresh package lists

6. **Auto-Install** (previously enabled): On app startup, the bootstrap would auto-install in the background. This was disabled because it could interfere with the first-launch experience. Users now install manually from Settings > Package Manager.

## The `pkg` Shell Command

A `pkg` shell script is written to `$PREFIX/bin/pkg` on every app startup. It provides a Termux-compatible CLI interface with additional repository management.

### Core Commands (delegated to `apt`)

| Command | Maps to | Description |
|---------|---------|-------------|
| `pkg install <pkg>` | `apt install <pkg>` | Install one or more packages |
| `pkg uninstall <pkg>` | `apt remove <pkg>` | Remove installed packages |
| `pkg update` | `apt update` | Refresh package lists from repositories |
| `pkg upgrade` | `apt upgrade` | Upgrade all upgradable packages |
| `pkg search <pattern>` | `apt search <pattern>` | Search package descriptions |
| `pkg show <pkg>` | `apt show <pkg>` | Show detailed package information |
| `pkg reinstall <pkg>` | `apt install --reinstall <pkg>` | Reinstall a package |

### Termux-Compatible Commands

| Command | Description |
|---------|-------------|
| `pkg list-installed` | Lists all installed packages via `dpkg -l` |
| `pkg files <pkg>` | Lists files owned by a package via `dpkg -L` |
| `pkg depends <pkg>` | Shows package dependencies via `apt depends` |

### Repository Management

The `pkg` command supports managing additional APT repositories through `$PREFIX/etc/apt/sources.list.d/`:

| Command | Description |
|---------|-------------|
| `pkg repo list` | Lists all configured repositories with their deb lines |
| `pkg add-repo <name>` | Interactive: prompts to add one of three known repos |
| `pkg add-repo <name> <url> <dist> [comp]` | Add a custom repository with explicit parameters |
| `pkg remove-repo <name>` | Remove a repository by name |

**Known Repositories:**

| Name | Repository URL | Purpose |
|------|---------------|---------|
| `x11` | `https://packages.termux.dev/apt/termux-x11/ x11 main` | GUI applications and X11 support |
| `root` | `https://packages.termux.dev/apt/termux-root/ root stable` | Packages requiring root privileges |
| `unstable` | `https://packages.termux.dev/apt/termux-unstable/ unstable main` | Bleeding-edge/unstable packages |

All repo commands require the bootstrap to be installed first. After adding/removing a repo, the user is prompted to run `pkg update`.

### Environment

Commands run inside the Termux environment use these environment variables:
- `PATH`: `$PREFIX/bin:/system/bin:/system/xbin:/vendor/bin`
- `PREFIX`: The Termux prefix directory
- `HOME`: The app's private home directory
- `LD_LIBRARY_PATH`: `$PREFIX/lib`
- `TMPDIR`: `$PREFIX/tmp`
- `TERM`: `xterm-256color`

The environment is cleared (`env_clear()`) before spawning child processes to prevent host environment leakage.

## Tauri Commands (UI Integration)

The following Rust functions are exposed as Tauri commands for the React frontend:

- `termux_is_installed()` — Check if the bootstrap is installed (presence of `$PREFIX/bin/apt`)
- `termux_bootstrap_status()` — Returns `BootstrapStatus` with install state, architecture, prefix path, and install-in-progress flag
- `termux_install_bootstrap(app, on_event)` — Installs the bootstrap with progress reporting via a Tauri `Channel`
- `termux_run_apt(args)` — Runs an arbitrary apt command inside the Termux environment (spawned on a blocking thread)
- `termux_list_packages()` — Parses `$PREFIX/var/lib/dpkg/status` and returns installed packages with name, version, and description

## Settings UI

The **Package Manager** section in Settings (`PackagesSection.tsx`) provides:
- Bootstrap status indicator (installed / not installed / arch / prefix path)
- "Install Bootstrap" button with progress bar and live log output
- Quick action buttons: `apt update`, `apt upgrade`, `apt list --upgradable`
- Common package install buttons: openssh, git, python, nodejs, build-essential, vim
- Custom apt command input field
- List of installed packages with version info

## Permission Recovery

After bootstrap installation and on every app startup, `fix_prefix_executables()` is called to ensure all executables have the execute bit set. This recovers from:
- Bootstrap zip entries lacking Unix mode metadata
- Android filesystems that lose sticky bits across app restarts
- Backup/restore cycles that strip permissions
- OEM "optimizations" that clear execute bits

The `.shrc` script also includes a shell-level fallback that `chmod +x` all files in `$PREFIX/bin` on every interactive shell start.
