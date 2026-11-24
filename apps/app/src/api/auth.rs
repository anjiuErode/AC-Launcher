use crate::api::Result;
use anyhow::Context;
use chrono::{Duration, Utc};
use tauri::plugin::TauriPlugin;
use tauri::{Manager, Runtime, UserAttentionType};
use theseus::prelude::*;

pub fn init<R: Runtime>() -> TauriPlugin<R> {
    tauri::plugin::Builder::<R>::new("auth")
        .invoke_handler(tauri::generate_handler![
            check_reachable,
            login,
            remove_user,
            get_default_user,
            set_default_user,
            get_users,
            get_offline_users,
            create_offline_user,
            get_external_users,
            discover_external_server,
            begin_external_login,
            choose_external_profile,
            discard_external_login,
        ])
        .build()
}

/// Checks if the authentication servers are reachable.
#[tauri::command]
pub async fn check_reachable() -> Result<()> {
    minecraft_auth::check_reachable().await?;
    Ok(())
}

/// Authenticate a user with Hydra - part 1
/// This begins the authentication flow quasi-synchronously, returning a URL to visit (that the user will sign in at)
#[tauri::command]
pub async fn login<R: Runtime>(
    app: tauri::AppHandle<R>,
) -> Result<Option<Credentials>> {
    let flow = minecraft_auth::begin_login().await?;

    let start = Utc::now();

    if let Some(window) = app.get_webview_window("signin") {
        window.close()?;
    }

    let window = tauri::WebviewWindowBuilder::new(
        &app,
        "signin",
        tauri::WebviewUrl::External(
            flow.auth_request_uri
                .parse()
                .context("error parsing auth redirect URL")?,
        ),
    )
    .title("Sign into Modrinth")
    .always_on_top(true)
    .min_inner_size(500.0, 500.0)
    .inner_size(1000.0, 700.0)
    .focused(true)
    .center()
    .build()?;

    window.request_user_attention(Some(UserAttentionType::Critical))?;

    while (Utc::now() - start) < Duration::minutes(10) {
        if window.title().is_err() {
            // user closed window, cancelling flow
            return Ok(None);
        }

        if window
            .url()?
            .as_str()
            .starts_with("https://login.live.com/oauth20_desktop.srf")
            && let Some((_, code)) =
                window.url()?.query_pairs().find(|x| x.0 == "code")
        {
            window.close()?;
            let val = minecraft_auth::finish_login(&code.clone(), flow).await?;

            return Ok(Some(val));
        }

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }

    window.close()?;
    Ok(None)
}

#[tauri::command]
pub async fn remove_user(user: uuid::Uuid) -> Result<()> {
    Ok(minecraft_auth::remove_user(user).await?)
}

#[tauri::command]
pub async fn get_default_user() -> Result<Option<uuid::Uuid>> {
    Ok(minecraft_auth::get_default_user().await?)
}

#[tauri::command]
pub async fn set_default_user(user: uuid::Uuid) -> Result<()> {
    Ok(minecraft_auth::set_default_user(user).await?)
}

/// Get a copy of the list of all user credentials
#[tauri::command]
pub async fn get_users() -> Result<Vec<Credentials>> {
    Ok(minecraft_auth::users().await?)
}

#[tauri::command]
pub async fn get_offline_users() -> Result<Vec<minecraft_auth::OfflineAccount>> {
    Ok(minecraft_auth::offline_accounts().await?)
}

#[tauri::command]
pub async fn create_offline_user(name: &str) -> Result<minecraft_auth::OfflineAccount> {
    Ok(minecraft_auth::add_offline_account(name).await?)
}

#[tauri::command]
pub async fn get_external_users() -> Result<Vec<minecraft_auth::ExternalAccount>> {
    Ok(minecraft_auth::external_accounts().await?)
}

#[tauri::command]
pub async fn discover_external_server(address: &str) -> Result<minecraft_auth::ExternalAuthServer> {
    Ok(minecraft_auth::discover_external_server(address).await?)
}

#[tauri::command]
pub async fn begin_external_login(api_root: &str, username: &str, password: &str) -> Result<minecraft_auth::ExternalLogin> {
    Ok(minecraft_auth::begin_external_login(api_root, username, password).await?)
}

#[tauri::command]
pub async fn choose_external_profile(login_id: uuid::Uuid, profile_id: uuid::Uuid) -> Result<()> {
    Ok(minecraft_auth::choose_external_profile(login_id, profile_id).await?)
}

#[tauri::command]
pub async fn discard_external_login(login_id: uuid::Uuid) -> Result<()> {
    minecraft_auth::discard_external_login(login_id).await;
    Ok(())
}
