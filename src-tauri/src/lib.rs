mod api;
mod cli_manager;
mod codex_localization;
mod commands;
mod config_writer;
mod diagnostics;
mod proxy;
mod secret_store;
mod state;
mod updater;

use std::sync::Arc;

use tauri::{Emitter, Manager};

use state::{load_store, save_store, AppState, DEFAULT_HOSTS, SITE_HOST};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .setup(|app| {
            let app_dir = app
                .path()
                .app_data_dir()
                .unwrap_or_else(|_| std::path::PathBuf::from("."));
            let _ = std::fs::create_dir_all(&app_dir);

            let mut store = load_store(&app_dir)?;
            // All built-in domains serve the same relay; legacy single-domain
            // installs now participate in automatic line selection too.
            store.hosts = DEFAULT_HOSTS.iter().map(|s| s.to_string()).collect();
            if store
                .settings
                .preferred_host
                .as_ref()
                .is_some_and(|host| !store.hosts.contains(host))
            {
                store.settings.preferred_host = None;
            }
            // New installations and legacy installs without an explicit
            // preference start from the sub-prefixed relay domain.
            if store.settings.preferred_host.is_none() {
                store.settings.preferred_host = Some(SITE_HOST.to_string());
                let _ = save_store(&app_dir, &store);
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
            });
            app.manage(app_state);
            use tauri::menu::{Menu, MenuItem};
            use tauri::tray::TrayIconBuilder;
            let show = MenuItem::with_id(app, "show", "显示窗口", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "退出客户端", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show, &quit])?;
            let mut tray = TrayIconBuilder::new()
                .menu(&menu)
                .on_menu_event(|app, event| match event.id().as_ref() {
                    "show" => {
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                    "quit" => {
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                        let _ = app.emit("request-quit", ());
                    }
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let tauri::tray::TrayIconEvent::Click { .. } = event {
                        if let Some(window) = tray.app_handle().get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
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
            commands::detect_clis,
            commands::install_cli,
            commands::restart_codex,
            commands::codex_localization,
            commands::run_diagnostics,
            commands::check_update,
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
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
