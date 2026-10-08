mod api;
mod chatgpt_proxy;
mod cli_manager;
mod codex_desktop;
mod codex_desktop_download;
mod codex_enhancement;
mod codex_inject;
mod codex_localization;
mod codex_sessions;
mod codexplusplus;
mod commands;
mod computer_tools;
mod config_writer;
mod desktop_features;
mod desktop_locale;
mod diagnostics;
mod model_probe;
mod provider_manager;
mod proxy;
mod secret_store;
mod sidebar_delete;
mod startup;
mod state;
mod updater;

use std::sync::Arc;

#[cfg(target_os = "macos")]
use tauri::RunEvent;
use tauri::{Emitter, Manager};

use state::{load_store, save_store, AppState, DEFAULT_HOSTS, SITE_HOST};

fn restore_main_window<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    // Embedded site views make main a multi-webview native window.
    // get_webview_window only resolves windows containing one webview.
    if let Some(window) = app.get_window("main") {
        if let Err(error) = window.show() {
            tracing::error!("Cannot show main window: {error}");
        }
        if let Err(error) = window.unminimize() {
            tracing::error!("Cannot restore main window: {error}");
        }
        #[cfg(all(target_os = "macos", not(test)))]
        {
            // A hidden macOS app can keep its window visible but inactive.
            // Activate the bundle before focusing the webview.
            let _ = std::process::Command::new("/usr/bin/osascript")
                .args([
                    "-e",
                    r#"tell application id "cc.jokerdeck.client" to activate"#,
                ])
                .output();
        }
        if let Err(error) = window.set_focus() {
            tracing::error!("Cannot focus main window: {error}");
        }
    } else {
        tracing::error!("Main native window not found");
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init();

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            restore_main_window(app);
        }))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_notification::init())
        .setup(|app| {
            let app_dir = app
                .path()
                .app_data_dir()
                .unwrap_or_else(|_| std::path::PathBuf::from("."));
            let _ = std::fs::create_dir_all(&app_dir);

            let mut store = load_store(&app_dir)?;
            computer_tools::set_enabled(store.settings.native_computer_tools);
            #[cfg(not(target_os = "android"))]
            match config_writer::recover_stale_proxy(store.settings.proxy_port) {
                Ok(true) => {
                    if let Err(error) = startup::set_enabled(false) {
                        tracing::warn!("无法撤销代理恢复登录启动项：{error}");
                    }
                }
                Ok(false) => {}
                Err(error) => tracing::error!("无法恢复上次运行残留的本地代理配置：{error}"),
            }
            // All built-in domains serve the same relay; legacy single-domain
            // installs now participate in automatic line selection too.
            store.hosts = DEFAULT_HOSTS.iter().map(|s| s.to_string()).collect();
            // Older clients persisted the canonical host as an implicit default.
            // Migrate it once so automatic mode can choose the measured fastest host.
            if !store.settings.host_selection_migrated {
                if store.settings.preferred_host.as_deref() == Some(SITE_HOST) {
                    store.settings.preferred_host = None;
                }
                store.settings.host_selection_migrated = true;
                save_store(&app_dir, &store)?;
            }
            if store
                .settings
                .preferred_host
                .as_ref()
                .is_some_and(|host| !store.hosts.contains(host))
            {
                store.settings.preferred_host = None;
            }
            let proxy = proxy::shared_with(store.hosts.clone(), store.settings.auto_fallback);
            {
                let mut status = proxy.try_write().expect("new proxy state is unlocked");
                status.preferred_host = store.settings.preferred_host.clone();
                status.active_host_idx = status
                    .preferred_host
                    .as_ref()
                    .and_then(|host| status.hosts.iter().position(|entry| &entry.host == host))
                    .unwrap_or(0);
            }

            let http = reqwest::Client::builder()
                .connect_timeout(std::time::Duration::from_secs(8))
                .user_agent(concat!("jokerdeck-desktop/", env!("CARGO_PKG_VERSION")))
                // The GFW resets TLS handshakes by matching the SNI (server name)
                // of the relay's blocked domains. Disabling SNI dodges that RST:
                // we still connect to the domain's real IP and still verify the
                // presented cert against the URL host (the relay's wildcard/SAN
                // cert covers all its domains), so this is a transport-layer
                // evasion of censorship, not a downgrade of certificate checks.
                .tls_sni(false)
                .build()
                .expect("failed to build http client");

            let app_state: state::SharedState = Arc::new(AppState {
                http,
                app_dir,
                session: tokio::sync::RwLock::new(None),
                store: tokio::sync::RwLock::new(store),
                proxy,
                proxy_runtime: tokio::sync::Mutex::new(None),
                configuration_lock: tokio::sync::Mutex::new(()),
                chatgpt_proxy: tokio::sync::Mutex::new(None),
                bundled_chatgpt_core: app.path().resource_dir().ok().map(|dir| {
                    dir.join("chatgpt-core").join(if cfg!(windows) {
                        "mihomo.exe"
                    } else {
                        "mihomo"
                    })
                }),
            });
            desktop_features::start(app_state.clone())?;
            let browser_state = app_state.clone();
            tauri::async_runtime::spawn(async move {
                let enabled = browser_state
                    .store
                    .read()
                    .await
                    .settings
                    .native_browser_compatibility;
                codexplusplus::configure(enabled).await;
            });
            app.manage(app_state);
            use tauri::menu::{Menu, MenuItem};
            use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
            let show = MenuItem::with_id(app, "show", "显示窗口", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "退出客户端", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show, &quit])?;
            let mut tray = TrayIconBuilder::new()
                .menu(&menu)
                // macOS otherwise consumes the left click to open the menu and
                // never reaches the restore handler below.
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id().as_ref() {
                    "show" => {
                        restore_main_window(app);
                    }
                    "quit" => {
                        restore_main_window(app);
                        let _ = app.emit("request-quit", ());
                    }
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    // Left click restores the window; right click keeps its menu
                    // usable and the explicit Show action restores the window.
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        restore_main_window(tray.app_handle());
                    }
                });
            if let Some(icon) = app.default_window_icon() {
                tray = tray.icon(icon.clone());
            }
            tray.build(app)?;
            Ok(())
        })
        .on_window_event(|window, event| {
            if window.label() == "main" {
                if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    let _ = window.emit("request-quit", ());
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::login,
            commands::register,
            commands::public_auth_settings,
            commands::send_verify_code,
            commands::forgot_password,
            commands::reset_password,
            commands::submit_2fa,
            commands::logout,
            commands::get_account,
            commands::list_groups,
            commands::fetch_plaza,
            commands::fetch_group_models,
            commands::test_group_model,
            commands::read_tool_config,
            commands::apply_config,
            commands::proxy_status,
            commands::stop_proxy,
            commands::restore_config,
            commands::quit_app,
            commands::probe_hosts,
            commands::set_preferred_host,
            commands::set_auto_fallback,
            commands::set_extensions,
            commands::native_browser_status,
            commands::configure_native_browser,
            commands::computer_tools_status,
            commands::configure_computer_tools,
            commands::client_provider_policy,
            commands::set_client_provider_policy,
            commands::switch_external_provider,
            commands::detect_clis,
            commands::install_cli,
            commands::download_codex_desktop,
            commands::restart_codex,
            commands::chatgpt_proxy_status,
            commands::configure_chatgpt_subscription,
            commands::start_chatgpt_proxy,
            commands::codex_desktop_running,
            commands::stop_codex_desktop,
            commands::codex_localization,
            commands::list_codex_sessions,
            commands::read_codex_session,
            commands::codex_enhancement_status,
            commands::desktop_feature_status,
            commands::configure_desktop_features,
            commands::provider_sync_action,
            commands::enable_codex_marketplace,
            commands::register_codex_plugin_cache,
            commands::run_diagnostics,
            commands::check_update,
            commands::install_update,
            commands::get_bootstrap,
            commands::open_url,
            commands::open_devtools,
            commands::open_site,
            commands::show_site,
            commands::hide_site,
            commands::close_site,
            commands::save_login,
            commands::forget_password,
            commands::set_site_url,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            #[cfg(target_os = "macos")]
            if let RunEvent::Reopen { .. } = event {
                restore_main_window(app);
            }
            #[cfg(not(target_os = "macos"))]
            let _ = (app, event);
        });
}

#[cfg(all(test, target_os = "macos"))]
mod window_restore_tests {
    use super::*;

    #[test]
    fn restores_native_main_after_embedded_site_is_added() {
        let app = tauri::test::mock_builder()
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();
        let main = tauri::WebviewWindowBuilder::new(&app, "main", tauri::WebviewUrl::default())
            .build()
            .unwrap();
        main.as_ref()
            .window()
            .add_child(
                tauri::webview::WebviewBuilder::new("relay-site", tauri::WebviewUrl::default()),
                tauri::LogicalPosition::new(0., 0.),
                tauri::LogicalSize::new(640., 480.),
            )
            .unwrap();
        // The legacy lookup fails once the embedded site creates another webview.
        assert!(app.get_webview_window("main").is_none());
        let native = app.get_window("main").unwrap();
        native.hide().unwrap();
        restore_main_window(app.handle());
    }
}
