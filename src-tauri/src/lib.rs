pub mod modules;

#[cfg(target_os = "android")]
use modules::android_fs;
#[cfg(target_os = "android")]
use modules::termux_pkg;
use modules::{agent, fs, git, net, pty, secrets, shell, workspace};
use std::sync::Mutex;
use tauri::{Emitter, Manager, State};
#[cfg(not(any(target_os = "android", target_os = "ios")))]
use tauri::{WebviewUrl, WebviewWindowBuilder};
#[cfg(target_os = "macos")]
use tauri::{PhysicalPosition, WindowEvent};
#[cfg(not(any(target_os = "android", target_os = "ios")))]
use tauri_plugin_window_state::StateFlags;

/// Drained on first read so HMR / re-mounts can't replay the launch dir.
#[derive(Default)]
struct LaunchDir(Mutex<Option<String>>);

#[tauri::command]
fn get_launch_dir(state: State<'_, LaunchDir>) -> Option<String> {
    state.0.lock().expect("LaunchDir mutex poisoned").take()
}

fn parse_launch_dir() -> Option<String> {
    for arg in std::env::args().skip(1) {
        if arg.starts_with('-') {
            continue;
        }
        let Ok(canon) = std::fs::canonicalize(&arg) else {
            continue;
        };
        if !canon.is_dir() {
            continue;
        }
        return Some(crate::modules::fs::to_canon(&canon));
    }
    None
}

#[cfg(not(target_os = "android"))]
#[tauri::command]
async fn open_settings_window(app: tauri::AppHandle, tab: Option<String>) -> Result<(), String> {
    let url_path = match tab.as_deref() {
        Some(t) if !t.is_empty() => format!("settings.html?tab={}", t),
        _ => "settings.html".to_string(),
    };

    if let Some(window) = app.get_webview_window("settings") {
        #[cfg(not(target_os = "android"))]
        let _ = window.set_always_on_top(true);
        let _ = window.show();
        let _ = window.set_focus();
        if let Some(t) = tab.as_deref().filter(|s| !s.is_empty()) {
            let _ = window.emit("terax:settings-tab", t);
        }
        return Ok(());
    }

    let builder = WebviewWindowBuilder::new(&app, "settings", WebviewUrl::App(url_path.into()))
        .title("Settings")
        .inner_size(900.0, 700.0)
        .min_inner_size(820.0, 620.0)
        .resizable(true)
        .visible(false);
    #[cfg(not(target_os = "android"))]
    let builder = builder.always_on_top(true);

    #[cfg(not(target_os = "macos"))]
    let builder = if let Some(main) = app.get_webview_window("main") {
        builder.parent(&main).map_err(|e| e.to_string())?
    } else {
        builder
    };

    #[cfg(target_os = "macos")]
    let builder = builder
        .title_bar_style(tauri::TitleBarStyle::Overlay)
        .hidden_title(true);

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    let builder = builder.decorations(false).transparent(true);

    let window = builder.build().map_err(|e| e.to_string())?;

    #[cfg(target_os = "linux")]
    {
        let _ = window.set_decorations(false);
    }

    #[cfg(target_os = "macos")]
    if let Some(main) = app.get_webview_window("main") {
        if let (Ok(main_pos), Ok(main_size), Ok(settings_size)) = (
            main.outer_position(),
            main.outer_size(),
            window.outer_size(),
        ) {
            let x = main_pos.x
                + ((main_size.width as i32).saturating_sub(settings_size.width as i32)) / 2;
            let y = main_pos.y
                + ((main_size.height as i32).saturating_sub(settings_size.height as i32)) / 2;
            let _ = window.set_position(PhysicalPosition::new(x, y));
        } else {
            let _ = window.center();
        }
    }

    Ok(())
}

#[cfg(target_os = "android")]
#[tauri::command]
async fn open_settings_window(app: tauri::AppHandle, tab: Option<String>) -> Result<(), String> {
    if let Some(main) = app.get_webview_window("main") {
        let _ = main.emit("terax:open-settings", tab);
    }
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let cli_dir = parse_launch_dir();
    workspace::init_launch_cwd(cli_dir.as_deref());

    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_store::Builder::new().build())
        .plugin(tauri_plugin_os::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(
            tauri_plugin_log::Builder::new()
                .level(tauri_plugin_log::log::LevelFilter::Info)
                .build(),
        )
        .plugin(tauri_plugin_opener::init());

    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    let builder = builder.plugin(tauri_plugin_updater::Builder::new().build());
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    let builder = builder.plugin(
        tauri_plugin_window_state::Builder::new()
            .with_state_flags(StateFlags::all() & !StateFlags::VISIBLE)
            .build(),
    );
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    let builder = builder.plugin(tauri_plugin_autostart::Builder::new().build());

    let builder = builder
        .setup(|_app| {
            #[cfg(target_os = "macos")]
            if let Some(main) = _app.get_webview_window("main") {
                let handle = _app.handle().clone();
                main.on_window_event(move |event| {
                    if matches!(
                        event,
                        WindowEvent::CloseRequested { .. } | WindowEvent::Destroyed
                    ) {
                        if let Some(settings) = handle.get_webview_window("settings") {
                            let _ = settings.close();
                        }
                    }
                });
            }
            // Android: materialize the Termux-style home + profile *before*
            // any PTY/workspace command is invoked, so the first tab the user
            // opens already has a usable filesystem. Also re-authorize the
            // home + prefix as workspace roots; the eager `bootstrap_registry`
            // ran before we had an AppHandle, so it couldn't see the Android
            // paths and only got the empty `dirs::home_dir()` fallback.
            #[cfg(target_os = "android")]
            {
                match android_fs::init(&_app.handle()) {
                    Ok(home) => {
                        log::info!("android home dir: {}", home.display());
                        let registry = _app.state::<workspace::WorkspaceRegistry>();
                        let _ = registry.authorize(&home);
                        if let Some(prefix) = android_fs::prefix() {
                            let _ = registry.authorize(prefix);
                        }
                    }
                    Err(e) => log::error!("android_fs::init failed: {e}"),
                }
                // Auto-install Termux bootstrap in the background so
                // apt/pkg are available without manual setup.
                let handle = _app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    termux_pkg::auto_install(&handle).await;
                });
            }
            Ok(())
        })
        .manage(pty::PtyState::default())
        .manage(shell::ShellState::default());

    let builder = builder.manage(secrets::SecretsState::default());

    let builder = builder
        .manage(fs::watch::FsWatchState::default())
        .manage({
            let registry = workspace::WorkspaceRegistry::default();
            workspace::bootstrap_registry(&registry);
            if let Some(ref launch_dir) = cli_dir {
                let _ = registry.authorize(launch_dir);
            }
            registry
        })
        .manage(LaunchDir(Mutex::new(cli_dir)))
        .invoke_handler(tauri::generate_handler![
            pty::pty_open,
            pty::pty_write,
            pty::pty_resize,
            pty::pty_close,
            pty::pty_close_all,
            pty::pty_has_foreground_process,
            fs::tree::list_subdirs,
            fs::tree::fs_read_dir,
            fs::file::fs_read_file,
            fs::file::fs_write_file,
            fs::file::fs_write_file_base64,
            fs::file::fs_stat,
            fs::file::fs_canonicalize,
            fs::mutate::fs_create_file,
            fs::mutate::fs_create_dir,
            fs::mutate::fs_rename,
            fs::mutate::fs_delete,
            fs::watch::fs_watch_add,
            fs::watch::fs_watch_remove,
            fs::search::fs_search,
            fs::search::fs_list_files,
            fs::grep::fs_grep,
            fs::grep::fs_glob,
            git::commands::git_resolve_repo,
            git::commands::git_panel_snapshot,
            git::commands::git_status,
            git::commands::git_diff,
            git::commands::git_diff_content,
            git::commands::git_stage,
            git::commands::git_unstage,
            git::commands::git_discard,
            git::commands::git_commit,
            git::commands::git_fetch,
            git::commands::git_pull_ff_only,
            git::commands::git_push,
            git::commands::git_log,
            git::commands::git_show_commit,
            git::commands::git_commit_files,
            git::commands::git_commit_file_diff,
            git::commands::git_remote_url,
            shell::shell_run_command,
            shell::shell_session_open,
            shell::shell_session_run,
            shell::shell_session_close,
            shell::shell_bg_spawn,
            shell::shell_bg_logs,
            shell::shell_bg_kill,
            shell::shell_bg_list,
            workspace::wsl_list_distros,
            workspace::wsl_default_distro,
            workspace::wsl_home,
            workspace::workspace_authorize,
            workspace::workspace_current_dir,
            get_launch_dir,
            open_settings_window,
            agent::agent_enable_claude_hooks,
            agent::agent_claude_hooks_status,
            secrets::secrets_get,
            secrets::secrets_set,
            secrets::secrets_delete,
            secrets::secrets_get_all,
            net::lm_ping,
            net::ai_http_request,
            net::ai_http_stream,
            #[cfg(target_os = "android")]
            android_fs::android_home_dir,
            #[cfg(target_os = "android")]
            android_fs::android_init_home,
            #[cfg(target_os = "android")]
            android_fs::android_paths,
            #[cfg(target_os = "android")]
            termux_pkg::termux_is_installed,
            #[cfg(target_os = "android")]
            termux_pkg::termux_bootstrap_status,
            #[cfg(target_os = "android")]
            termux_pkg::termux_install_bootstrap,
            #[cfg(target_os = "android")]
            termux_pkg::termux_run_apt,
            #[cfg(target_os = "android")]
            termux_pkg::termux_list_packages,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
