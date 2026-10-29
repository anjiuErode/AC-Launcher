#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]
#![recursion_limit = "256"]

use anyhow::Context;
use native_dialog::{DialogBuilder, MessageLevel};
use std::env;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{Listener, Manager};
use tauri_plugin_fs::FsExt;
use theseus::prelude::*;

mod api;
mod guild;

#[cfg(target_os = "macos")]
mod macos;

#[cfg(feature = "updater")]
mod updater_impl;
#[cfg(not(feature = "updater"))]
mod updater_impl_noop;

// Should be called in launcher initialization
#[tracing::instrument(skip_all)]
#[tauri::command]
async fn initialize_state(
    app: tauri::AppHandle,
    events: tauri::ipc::Channel<tauri::ipc::InvokeResponseBody>,
) -> api::Result<()> {
    tracing::info!("Initializing app event state...");
    theseus::EventState::init(app.clone(), events).await?;

    tracing::info!("Initializing app state...");
    State::init(app.config().identifier.clone()).await?;

    let state = State::get().await?;
    app.asset_protocol_scope()
        .allow_directory(state.directories.caches_dir(), true)?;
    app.asset_protocol_scope()
        .allow_directory(state.directories.caches_dir().join("icons"), true)?;
    app.asset_protocol_scope()
        .allow_directory(state.directories.icon_dir(), true)?;
    app.fs_scope()
        .allow_directory(state.directories.instances_dir(), true)?;

    Ok(())
}

// Should be call once Vue has mounted the app
#[tracing::instrument(skip_all)]
#[tauri::command]
fn show_window(app: tauri::AppHandle) {
    let win = app.get_window("main").unwrap();
    if let Err(e) = win.show() {
        DialogBuilder::message()
            .set_level(MessageLevel::Error)
            .set_title("Initialization error")
            .set_text(format!(
                "Cannot display application window due to an error:\n{e}"
            ))
            .alert()
            .show()
            .unwrap();
        panic!("cannot display application window")
    } else if let Err(error) = win.set_focus() {
        // 窗口已显示但抢不到焦点（例如被前台全屏程序压住），不该静默吞掉
        tracing::warn!("Failed to focus the main window after showing it: {error}");
    }
}

#[tauri::command]
fn is_dev() -> bool {
    cfg!(debug_assertions)
}

#[tauri::command]
fn are_updates_enabled() -> bool {
    cfg!(feature = "updater")
        && env::var("MODRINTH_EXTERNAL_UPDATE_PROVIDER").is_err()
}

#[cfg(feature = "updater")]
pub use updater_impl::*;

#[cfg(not(feature = "updater"))]
pub use updater_impl_noop::*;

// Toggles decorations
#[tauri::command]
async fn toggle_decorations(b: bool, window: tauri::Window) -> api::Result<()> {
    window
        .set_decorations(b)
        .context("failed to toggle decorations")?;
    Ok(())
}

#[tauri::command]
fn restart_app(app: tauri::AppHandle) {
    app.restart();
}

#[derive(Default)]
struct RestartAfterPendingUpdate(AtomicBool);

#[tauri::command]
async fn set_restart_after_pending_update(
    should_restart: bool,
    restart: tauri::State<'_, RestartAfterPendingUpdate>,
) -> api::Result<()> {
    restart.0.store(should_restart, Ordering::Relaxed);
    Ok(())
}

// if Tauri app is called with arguments, then those arguments will be treated as commands
// ie: deep links or filepaths for .mrpacks
/// 硬件加速配置文件路径（与 Tauri 的 appConfigDir 保持一致）
fn gpu_config_path(identifier: &str) -> Option<std::path::PathBuf> {
    #[cfg(target_os = "windows")]
    let base = std::env::var_os("APPDATA").map(std::path::PathBuf::from);
    #[cfg(target_os = "macos")]
    let base = std::env::var_os("HOME")
        .map(|home| std::path::PathBuf::from(home).join("Library/Application Support"));
    #[cfg(all(unix, not(target_os = "macos")))]
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|home| std::path::PathBuf::from(home).join(".config"))
        });

    base.map(|dir| dir.join(identifier).join("gpu.json"))
}

fn read_hardware_acceleration(identifier: &str) -> bool {
    let Some(path) = gpu_config_path(identifier) else {
        return true;
    };
    let Ok(raw) = std::fs::read_to_string(path) else {
        return true;
    };
    serde_json::from_str::<serde_json::Value>(&raw)
        .ok()
        .and_then(|value| value.get("hardware_acceleration").and_then(|v| v.as_bool()))
        .unwrap_or(true)
}

fn write_hardware_acceleration(identifier: &str, enabled: bool) -> Result<(), String> {
    let path = gpu_config_path(identifier)
        .ok_or_else(|| "无法定位应用配置目录".to_string())?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let payload = serde_json::json!({ "hardware_acceleration": enabled });
    std::fs::write(&path, payload.to_string()).map_err(|e| e.to_string())
}

/// 必须在创建 WebView 之前调用，WebView2 只读取一次该环境变量
fn apply_hardware_acceleration(identifier: &str) {
    let enabled = read_hardware_acceleration(identifier);
    #[cfg(target_os = "windows")]
    {
        let args = if enabled {
            "--ignore-gpu-blocklist --enable-gpu-rasterization --enable-zero-copy \
             --enable-features=CanvasOopRasterization"
        } else {
            "--disable-gpu --disable-gpu-compositing --disable-software-rasterizer"
        };
        // SAFETY: 单线程启动阶段调用，早于 WebView2 初始化
        unsafe { std::env::set_var("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS", args) };
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = enabled;
    }
    tracing::info!("Hardware acceleration preference: {enabled}");
}

#[tauri::command]
fn get_hardware_acceleration(app: tauri::AppHandle) -> bool {
    read_hardware_acceleration(&app.config().identifier)
}

#[tauri::command]
fn set_hardware_acceleration(app: tauri::AppHandle, enabled: bool) -> Result<(), String> {
    write_hardware_acceleration(&app.config().identifier, enabled)
}

// ============ 网络代理设置 ============

fn app_config_dir_for(identifier: &str) -> Option<std::path::PathBuf> {
    #[cfg(target_os = "windows")]
    let base = std::env::var_os("APPDATA").map(std::path::PathBuf::from);
    #[cfg(target_os = "macos")]
    let base = std::env::var_os("HOME")
        .map(|home| std::path::PathBuf::from(home).join("Library/Application Support"));
    #[cfg(all(unix, not(target_os = "macos")))]
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|home| std::path::PathBuf::from(home).join(".config"))
        });
    base.map(|dir| dir.join(identifier))
}

fn read_proxy_config(identifier: &str) -> theseus::ProxyConfig {
    app_config_dir_for(identifier)
        .map(|dir| dir.join("proxy.json"))
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

fn write_proxy_config(
    identifier: &str,
    config: &theseus::ProxyConfig,
) -> Result<(), String> {
    let dir = app_config_dir_for(identifier)
        .ok_or_else(|| "无法定位应用配置目录".to_string())?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let raw = serde_json::to_string_pretty(config).map_err(|e| e.to_string())?;
    std::fs::write(dir.join("proxy.json"), raw).map_err(|e| e.to_string())
}

#[tauri::command]
fn get_proxy_config(app: tauri::AppHandle) -> theseus::ProxyConfig {
    read_proxy_config(&app.config().identifier)
}

#[tauri::command]
fn set_proxy_config(
    app: tauri::AppHandle,
    mode: String,
    url: Option<String>,
) -> Result<(), String> {
    let mode: theseus::ProxyMode =
        serde_json::from_str(&format!("\"{mode}\""))
            .map_err(|_| format!("未知的代理模式: {mode}"))?;
    let config = theseus::ProxyConfig { mode, url };
    write_proxy_config(&app.config().identifier, &config)?;
    theseus::set_http_proxy_config(config);
    Ok(())
}

/// 打开官网授权页，等待本机回环回调完成登录
#[tauri::command]
async fn guild_begin_login(app: tauri::AppHandle) -> Result<guild::PublicSession, String> {
    guild::begin_login(&app.config().identifier).await
}

#[tauri::command]
fn guild_session(app: tauri::AppHandle) -> Option<guild::PublicSession> {
    guild::current(&app.config().identifier)
}

#[tauri::command]
fn guild_sign_out(app: tauri::AppHandle) -> Result<(), String> {
    guild::sign_out(&app.config().identifier)
}

/// 公会数据接口：令牌只在 Rust 侧使用，前端拿不到
#[tauri::command]
async fn guild_me(app: tauri::AppHandle) -> Result<guild::GuildMe, guild::GuildApiError> {
    guild::fetch_me(&app.config().identifier).await
}

#[tauri::command]
async fn guild_announcements(
    app: tauri::AppHandle,
    limit: Option<u32>,
) -> Result<Vec<guild::GuildAnnouncement>, guild::GuildApiError> {
    guild::fetch_announcements(&app.config().identifier, limit).await
}

/// 读取附件文本内容，供界面内联预览（不需要令牌）
#[tauri::command]
async fn guild_attachment_text(
    news_id: String,
    attachment_id: String,
) -> Result<String, guild::GuildApiError> {
    guild::fetch_attachment_text(&news_id, &attachment_id).await
}

#[tauri::command]
async fn guild_packs(
    app: tauri::AppHandle,
) -> Result<Vec<guild::GuildPack>, guild::GuildApiError> {
    guild::fetch_packs(&app.config().identifier).await
}

#[tauri::command]
async fn guild_servers(
    app: tauri::AppHandle,
) -> Result<Vec<guild::GuildServer>, guild::GuildApiError> {
    guild::fetch_servers(&app.config().identifier).await
}

/// 在线列表（只读）
#[tauri::command]
async fn guild_presence(
    app: tauri::AppHandle,
) -> Result<Vec<guild::GuildPresence>, guild::GuildApiError> {
    guild::fetch_presence_list(&app.config().identifier).await
}

/// 当前隐身偏好（默认可见）
#[tauri::command]
fn guild_presence_visible(app: tauri::AppHandle) -> bool {
    guild::presence_visible(&app.config().identifier)
}

/// 切换隐身。改完由界面立刻补报一次，让状态即时生效
#[tauri::command]
fn guild_set_presence_visible(app: tauri::AppHandle, visible: bool) -> Result<(), String> {
    guild::set_presence_visible(&app.config().identifier, visible)
}

/// 界面上报「现在在玩哪个实例」，后台据此每 30 秒心跳
#[tauri::command]
fn guild_set_presence_state(
    instance_name: String,
    mc_version: String,
    loader: String,
    running: bool,
) {
    guild::update_presence_state(guild::PresenceState {
        running,
        instance_name,
        mc_version,
        loader,
    });
}

/// 下载公会整合包到本地并校验（P4-2）
#[tauri::command]
async fn guild_download_pack(
    url: String,
    sha256: String,
    filename: String,
) -> Result<guild::GuildPackFile, guild::GuildApiError> {
    guild::download_pack(&url, &sha256, &filename).await
}

/// 探测公会整合包的格式与内嵌版本信息（P4-2）
#[tauri::command]
async fn guild_inspect_pack(path: String) -> Result<guild::PackInspection, guild::GuildApiError> {
    guild::inspect_pack(&path).await
}

/// 把整合包流式解压到暂存目录（P4-2）。先落 staging，成功后才提交进实例，
/// 避免包损坏或中途失败时留下半成品实例。
#[tauri::command]
async fn guild_extract_pack(
    path: String,
    root_prefix: String,
) -> Result<guild::PackStaging, guild::GuildApiError> {
    guild::extract_pack_to_staging(&path, &root_prefix).await
}

/// 把暂存内容提交进实例目录（P4-2）
#[tauri::command]
async fn guild_commit_pack(
    staging_dir: String,
    instance_id: String,
) -> Result<usize, guild::GuildApiError> {
    guild::commit_staging_to_instance(&staging_dir, &instance_id).await
}

/// 放弃暂存目录（P4-2）
#[tauri::command]
async fn guild_discard_pack(staging_dir: String) -> Result<(), guild::GuildApiError> {
    guild::discard_staging(&staging_dir).await
}

/// 立即上报一次（切换隐身、刚进游戏时用）
#[tauri::command]
async fn guild_report_presence(
    app: tauri::AppHandle,
) -> Result<bool, guild::GuildApiError> {
    guild::report_presence(&app.config().identifier).await
}

#[derive(serde::Serialize)]
struct NetProbe {
    label: String,
    url: String,
    ok: bool,
    status: Option<u16>,
    millis: u64,
    error: Option<String>,
}

/// 用给定的（尚未保存的）代理配置试连，用于设置页的「测试连接」
#[tauri::command]
async fn test_network(mode: String, url: Option<String>) -> Result<Vec<NetProbe>, String> {
    let mode: theseus::ProxyMode =
        serde_json::from_str(&format!("\"{mode}\""))
            .map_err(|_| format!("未知的代理模式: {mode}"))?;
    let config = theseus::ProxyConfig { mode, url };
    let client = theseus::client_builder_with(&config)
        .build()
        .map_err(|e| e.to_string())?;

    let targets = [
        ("Modrinth API", "https://api.modrinth.com/"),
        ("Microsoft 登录", "https://login.live.com/"),
    ];

    let mut probes = Vec::with_capacity(targets.len());
    for (label, target) in targets {
        let started = std::time::Instant::now();
        match client.get(target).send().await {
            Ok(resp) => {
                let status = resp.status();
                probes.push(NetProbe {
                    label: label.to_string(),
                    url: target.to_string(),
                    ok: status.is_success() || status.is_redirection(),
                    status: Some(status.as_u16()),
                    millis: started.elapsed().as_millis() as u64,
                    error: None,
                });
            }
            Err(err) => probes.push(NetProbe {
                label: label.to_string(),
                url: target.to_string(),
                ok: false,
                status: None,
                millis: started.elapsed().as_millis() as u64,
                error: Some(err.to_string()),
            }),
        }
    }
    Ok(probes)
}

fn main() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("failed to create app async runtime");
    tauri::async_runtime::set(runtime.handle().clone());

    #[cfg(feature = "export-app-events")]
    theseus::export_app_event_bindings(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../app-frontend/src/generated/app-events"),
    )
    .expect("failed to export app event TypeScript bindings");

    /*
        tracing is set basd on the environment variable RUST_LOG=xxx, depending on the amount of logs to show
            ERROR > WARN > INFO > DEBUG > TRACE
        eg. RUST_LOG=info will show info, warn, and error logs
            RUST_LOG="theseus=trace" will show *all* messages but from theseus only (and not dependencies using similar crates)
            RUST_LOG="theseus=trace" will show *all* messages but from theseus only (and not dependencies using similar crates)

        Error messages returned to Tauri will display as traced error logs if they return an error.
        This will also include an attached span trace if the error is from a tracing error, and the level is set to info, debug, or trace

        on unix:
            RUST_LOG="theseus=trace" {run command}

    */

    let tauri_context = tauri::generate_context!();

    apply_hardware_acceleration(&tauri_context.config().identifier);

    let _log_guard = theseus::start_logger(&tauri_context.config().identifier);

    tracing::info!("Initialized tracing subscriber. Loading Modrinth App!");

    // 代理配置必须在任何 HTTP 客户端首次构建前注入
    let proxy_config = read_proxy_config(&tauri_context.config().identifier);
    tracing::info!("Loaded network proxy preference: {:?}", proxy_config);
    theseus::set_http_proxy_config(proxy_config);

    let mut builder = tauri::Builder::default();

    #[cfg(target_os = "macos")]
    {
        builder = builder
            .menu(macos::menu::create)
            .on_menu_event(macos::menu::handle_event);
    }

    #[cfg(feature = "updater")]
    {
        use tauri_plugin_http::reqwest::header::{HeaderValue, USER_AGENT};
        use theseus::launcher_user_agent;
        builder = builder.plugin(
            tauri_plugin_updater::Builder::new()
                .header(
                    USER_AGENT,
                    HeaderValue::from_str(&launcher_user_agent()).unwrap(),
                )
                .unwrap()
                .build(),
        );
    }

    builder = builder
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            if let Some(payload) = args.get(1) {
                tracing::info!("Handling command-line deep link");
                let payload = payload.clone();
                tauri::async_runtime::spawn(api::utils::handle_command(
                    payload,
                ));
            }

            // 重复启动时必须把已有窗口「还原并前置」：
            // 只 set_focus 对隐藏窗口无效（关闭到托盘后就是隐藏状态），
            // 用户会以为没启动成功，再点几次也没反应。
            if let Some(win) = app.get_window("main") {
                match win.is_visible() {
                    Ok(true) => {}
                    Ok(false) => {
                        if let Err(error) = win.show() {
                            tracing::warn!(
                                "Failed to show the main window on second launch: {error}"
                            );
                        }
                    }
                    Err(error) => {
                        tracing::warn!(
                            "Failed to read main window visibility: {error}"
                        );
                    }
                }
                if let Err(error) = win.unminimize() {
                    tracing::warn!("Failed to unminimize the main window: {error}");
                }
                if let Err(error) = win.set_focus() {
                    tracing::warn!("Failed to focus the main window: {error}");
                }
            }
        }))
        .plugin(tauri_plugin_http::init())
        .plugin(tauri_plugin_os::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_opener::init());

    #[cfg(target_os = "windows")]
    {
        use windows::Win32::UI::WindowsAndMessaging::{
            SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
            SWP_NOZORDER, SetWindowPos,
        };

        // Refresh the hidden window's frame before window-state measures its client area.
        builder = builder.plugin(
            tauri::plugin::Builder::<_, ()>::new("window-frame")
                .on_window_ready(|window| {
                    if window.label() != "main" {
                        return;
                    }

                    let hwnd = match window.hwnd() {
                        Ok(hwnd) => hwnd,
                        Err(e) => {
                            tracing::warn!("Failed to get window handle: {e}");
                            return;
                        }
                    };

                    if let Err(e) = unsafe {
                        SetWindowPos(
                            hwnd,
                            None,
                            0,
                            0,
                            0,
                            0,
                            SWP_FRAMECHANGED
                                | SWP_NOACTIVATE
                                | SWP_NOMOVE
                                | SWP_NOSIZE
                                | SWP_NOZORDER,
                        )
                    } {
                        tracing::warn!("Failed to refresh window frame: {e}");
                    }
                })
                .build(),
        );
    }

    let window_state_flags = tauri_plugin_window_state::StateFlags::POSITION
        | tauri_plugin_window_state::StateFlags::SIZE
        | tauri_plugin_window_state::StateFlags::MAXIMIZED;
    #[cfg(target_os = "windows")]
    let window_state_flags =
        window_state_flags | tauri_plugin_window_state::StateFlags::DECORATIONS;

    builder = builder
        .plugin(
            tauri_plugin_window_state::Builder::default()
                .with_filename("app-window-state.json")
                .with_denylist(&["signin"])
                // Restoring VISIBLE would override the hidden startup window.
                .with_state_flags(window_state_flags)
                .build(),
        )
        .setup(|app| {
            #[cfg(target_os = "macos")]
            {
                let payload = macos::deep_link::get_or_init_payload(app);

                let mtx_copy = payload.payload;
                app.listen("deep-link://new-url", move |url| {
                    let mtx_copy_copy = mtx_copy.clone();
                    let request = url.payload().to_owned();

                    let actual_request =
                        serde_json::from_str::<Vec<String>>(&request)
                            .ok()
                            .map(|mut x| x.remove(0))
                            .unwrap_or(request);

                    tauri::async_runtime::spawn(async move {
                        tracing::info!("Handling macOS deep link");

                        let mut payload = mtx_copy_copy.lock().await;
                        if payload.is_none() {
                            *payload = Some(actual_request.clone());
                        }

                        let _ =
                            api::utils::handle_command(actual_request).await;
                    });
                });
            };

            #[cfg(not(target_os = "macos"))]
            app.listen("deep-link://new-url", |url| {
                let payload = url.payload().to_owned();
                tracing::info!("Handling deep link");
                tauri::async_runtime::spawn(api::utils::handle_command(
                    payload,
                ));
            });

            // 启动看门狗：窗口配置是 visible=false，正常由前端 onMounted 调 show_window。
            // 一旦前端在 setup 阶段抛错（历史上出现过注入失败），onMounted 永远不会执行，
            // 进程活着但窗口永不出现。这里兜底显示，让用户至少能看到界面或错误。
            {
                let handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_secs(15)).await;
                    let Some(win) = handle.get_window("main") else {
                        tracing::warn!(
                            "Startup watchdog found no main window to recover"
                        );
                        return;
                    };
                    match win.is_visible() {
                        Ok(true) => {}
                        Ok(false) => {
                            tracing::warn!(
                                "Main window is still hidden 15s after startup; forcing it visible (frontend likely failed to mount)"
                            );
                            if let Err(error) = win.show() {
                                tracing::error!(
                                    "Startup watchdog failed to show the main window: {error}"
                                );
                            }
                            if let Err(error) = win.set_focus() {
                                tracing::warn!(
                                    "Startup watchdog failed to focus the main window: {error}"
                                );
                            }
                        }
                        Err(error) => {
                            tracing::warn!(
                                "Startup watchdog could not read window visibility: {error}"
                            );
                        }
                    }
                });
            }

            // 公会在线心跳：界面负责推「在玩哪个实例」，这里负责每 30 秒上报一次。
            // 心跳放后台而不是 webview，是因为启动游戏后启动器默认最小化，
            // 前端的 setInterval 会被节流甚至暂停。
            {
                let handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    let mut ticker = tokio::time::interval(
                        std::time::Duration::from_secs(30),
                    );
                    ticker.set_missed_tick_behavior(
                        tokio::time::MissedTickBehavior::Delay,
                    );
                    loop {
                        ticker.tick().await;
                        let identifier = handle.config().identifier.clone();
                        if guild::current(&identifier).is_none() {
                            continue;
                        }
                        // 401 已在内部清会话；其余错误（断网、429）静默，
                        // 心跳失败不该打扰用户
                        let _ = guild::report_presence(&identifier).await;
                    }
                });
            }

            #[cfg(not(target_os = "linux"))]
            if let Some(window) = app.get_window("main")
                && let Err(e) = window.set_shadow(true)
            {
                tracing::warn!("Failed to set window shadow: {e}");
            }

            Ok(())
        });

    builder = builder
        .plugin(api::auth::init())
        .plugin(api::mr_auth::init())
        .plugin(api::onboarding_checklist::init())
        .plugin(api::import::init())
        .plugin(api::install::init())
        .plugin(api::instance::init())
        .plugin(api::logs::init())
        .plugin(api::jre::init())
        .plugin(api::metadata::init())
        .plugin(api::minecraft_skins::init())
        .plugin(api::process::init())
        .plugin(api::reports::init())
        .plugin(api::settings::init())
        .plugin(api::shortcuts::init())
        .plugin(api::tags::init())
        .plugin(api::users::init())
        .plugin(api::utils::init())
        .plugin(api::cache::init())
        .plugin(api::files::init())
        .plugin(api::friends::init())
        .plugin(api::worlds::init())
        .manage(PendingUpdateData::default())
        .manage(RestartAfterPendingUpdate::default())
        .invoke_handler(tauri::generate_handler![
            initialize_state,
            is_dev,
            are_updates_enabled,
            get_update_size,
            enqueue_update_for_installation,
            remove_enqueued_update,
            set_restart_after_pending_update,
            toggle_decorations,
            show_window,
            restart_app,
            get_hardware_acceleration,
            set_hardware_acceleration,
            get_proxy_config,
            set_proxy_config,
            test_network,
            guild_begin_login,
            guild_session,
            guild_sign_out,
            guild_me,
            guild_announcements,
            guild_attachment_text,
            guild_packs,
            guild_servers,
            guild_presence,
            guild_presence_visible,
            guild_set_presence_visible,
            guild_set_presence_state,
            guild_report_presence,
            guild_download_pack,
            guild_inspect_pack,
            guild_extract_pack,
            guild_commit_pack,
            guild_discard_pack,
        ]);

    tracing::info!("Initializing app...");
    let app = builder.build(tauri_context);

    match app {
        Ok(app) => {
            app.run(|app, event| {
                #[cfg(not(any(feature = "updater", target_os = "macos")))]
                let _ = app;

                if matches!(&event, tauri::RunEvent::ExitRequested { .. })
                    && let Err(error) = tauri::async_runtime::block_on(
                        theseus::minecraft_skins::flush_pending_skin_change(),
                    )
                {
                    tracing::warn!(
                        "Failed to flush pending Minecraft skin change before exit: {error}"
                    );
                }

                #[cfg(feature = "updater")]
                if matches!(&event, tauri::RunEvent::Exit) {
                    let update_data = app.state::<PendingUpdateData>().inner();
                    let should_restart = app
                        .state::<RestartAfterPendingUpdate>()
                        .0
                        .load(Ordering::Relaxed);
                    if let Some((update, data)) = &*update_data.0.lock().unwrap()
                    {
                        fn set_changelog_toast(version: Option<String>) {
                            let toast_result: theseus::Result<()> = tauri::async_runtime::block_on(async move {
                                let mut settings = settings::get().await?;
                                settings.pending_update_toast_for_version = version;
                                settings::set(settings).await?;
                                Ok(())
                            });
                            if let Err(e) = toast_result {
                                tracing::warn!(
                                    "Failed to set pending_update_toast: {e}"
                                )
                            }
                        }

						let current_version = &app.package_info().version;
						let mut version_parts = update.version.split('.');
						let major = version_parts.next().and_then(|part| part.parse::<u64>().ok());
						let minor = version_parts.next().and_then(|part| part.parse::<u64>().ok());
						let is_major_update = major.zip(minor).is_some_and(|version| {
							version > (current_version.major, current_version.minor)
						});
						set_changelog_toast(is_major_update.then(|| update.version.clone()));
                        let update = if should_restart {
                            (**update).clone()
                        } else {
                            (**update).clone().restart_after_install(false)
                        };
                        match update.install(data) {
                            Ok(()) => {
                                if should_restart {
                                    tracing::info!(
                                        "Pending update installed successfully (version {}); restarting because user requested reload",
                                        update.version
                                    );
                                    app.restart();
                                } else {
                                    tracing::info!(
                                        "Pending update installed successfully (version {}); exiting without relaunch (user did not request reload)",
                                        update.version
                                    );
                                }
                            }
                            Err(e) => {
                                tracing::error!(
                                    "Pending update install failed (version {}): {e}",
                                    update.version
                                );
                                set_changelog_toast(None);

                                DialogBuilder::message()
                                    .set_level(MessageLevel::Error)
                                    .set_title("Update error")
                                    .set_text(format!("Failed to install update due to an error:\n{e}"))
                                    .alert()
                                    .show()
                                    .unwrap();
                            }
                        }
                    }
                }
                #[cfg(target_os = "macos")]
                if let tauri::RunEvent::Opened { urls } = event {
                    tracing::info!("Handling webview open {urls:?}");

                    let file = urls
                        .into_iter()
                        .find_map(|url| url.to_file_path().ok());

                    if let Some(file) = file {
                        let payload =
                            macos::deep_link::get_or_init_payload(app);

                        let mtx_copy = payload.payload;
                        let request = file.to_string_lossy().to_string();
                        tauri::async_runtime::spawn(async move {
                            let mut payload = mtx_copy.lock().await;
                            if payload.is_none() {
                                *payload = Some(request.clone());
                            }

                            let _ = api::utils::handle_command(request).await;
                        });
                    }
                }
            });
        }
        Err(e) => {
            tracing::error!("Error while running tauri application: {:?}", e);

            #[cfg(target_os = "windows")]
            {
                // tauri doesn't expose runtime errors, so matching a string representation seems like the only solution
                if format!("{e:?}").contains(
                    "Runtime(CreateWebview(WebView2Error(WindowsError",
                ) {
                    DialogBuilder::message()
                        .set_level(MessageLevel::Error)
                        .set_title("Initialization error")
                        .set_text("Your Microsoft Edge WebView2 installation is corrupt.\n\nMicrosoft Edge WebView2 is required to run Modrinth App.\n\nLearn how to repair it at https://support.modrinth.com/en/articles/8797765-corrupted-microsoft-edge-webview2-installation")
                        .alert()
                        .show()
                        .unwrap();

                    panic!("webview2 initialization failed")
                }
            }

            DialogBuilder::message()
                .set_level(MessageLevel::Error)
                .set_title("Initialization error")
                .set_text(format!(
                    "Cannot initialize application due to an error:\n{e:?}"
                ))
                .alert()
                .show()
                .unwrap();

            panic!("{1}: {:?}", e, "error while running tauri application")
        }
    }
}
