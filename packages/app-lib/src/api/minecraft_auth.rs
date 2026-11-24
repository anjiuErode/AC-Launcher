//! Authentication flow interface

use anyhow::Context;
use reqwest::StatusCode;

use crate::State;
use crate::state::{Credentials, MinecraftLoginFlow};
use crate::util::fetch::INSECURE_REQWEST_CLIENT;

mod alternate_accounts;
mod authlib_injector;
mod external_account;
mod external_secrets;
pub use alternate_accounts::{OfflineAccount, add_offline_account, offline_accounts};
pub(crate) use alternate_accounts::{active_alternate_id, offline_account};
pub use external_account::{
    ExternalAccount, ExternalAuthServer, ExternalLogin, begin_external_login,
    choose_external_profile, discard_external_login, discover_external_server,
    external_accounts,
};
pub(crate) use external_account::{external_launch_identity, external_logging_tokens};

#[tracing::instrument]
pub async fn check_reachable() -> crate::Result<()> {
    let resp = INSECURE_REQWEST_CLIENT
        .get("https://sessionserver.mojang.com/session/minecraft/hasJoined")
        .send()
        .await?;
    if resp.status() == StatusCode::NO_CONTENT {
        return Ok(());
    }
    resp.error_for_status()?;
    Ok(())
}

#[tracing::instrument]
pub async fn begin_login() -> crate::Result<MinecraftLoginFlow> {
    let state = State::get().await?;

    crate::state::login_begin(&state.pool).await
}

#[tracing::instrument]
pub async fn finish_login(
    code: &str,
    flow: MinecraftLoginFlow,
) -> crate::Result<Credentials> {
    let state = State::get().await?;

    let credentials =
        crate::state::login_finish(code, flow, &state.pool).await?;

    crate::onboarding_checklist::mark_logged_into_minecraft().await;

    Ok(credentials)
}

#[tracing::instrument]
pub async fn get_default_user() -> crate::Result<Option<uuid::Uuid>> {
    let state = State::get().await?;
    if let Some(id) = active_alternate_id(&state.pool).await? {
        return Ok(Some(id));
    }
    let user = Credentials::get_default_credential(&state.pool).await?;
    Ok(user.map(|user| user.offline_profile.id))
}

#[tracing::instrument]
pub async fn set_default_user(user: uuid::Uuid) -> crate::Result<()> {
    let state = State::get().await?;
    let mut transaction = state.pool.begin_with("BEGIN IMMEDIATE").await?;
    if alternate_accounts::select_offline_account(&mut transaction, user).await?
        || external_account::select_external_account(&mut transaction, user).await?
    {
        transaction.commit().await?;
        return Ok(());
    }
    transaction.rollback().await?;
    let users = Credentials::get_all(&state.pool).await?;
    let (_, mut user) = users
        .remove(&user)
        .context(format!("tried to get nonexistent user with ID {user}"))?;

    user.active = true;
    user.upsert(&state.pool).await?;
    let mut conn = state.pool.acquire().await?;
    alternate_accounts::clear_active(&mut conn).await?;

    Ok(())
}

/// Remove a user account from the database
#[tracing::instrument]
pub async fn remove_user(uuid: uuid::Uuid) -> crate::Result<()> {
    let state = State::get().await?;

    let mut transaction = state.pool.begin_with("BEGIN IMMEDIATE").await?;
    if alternate_accounts::remove_offline_account(&mut transaction, uuid).await?
        || external_account::remove_external_account(&mut transaction, uuid).await?
    {
        transaction.commit().await?;
        return Ok(());
    }
    transaction.rollback().await?;

    let users = Credentials::get_all(&state.pool).await?;

    if let Some((uuid, user)) = users.remove(&uuid) {
        Credentials::remove(uuid, &state.pool).await?;

        if user.active
            && let Some((_, mut user)) = users.into_iter().next()
        {
            user.active = true;
            user.upsert(&state.pool).await?;
        }
    }

    Ok(())
}

/// Get a copy of the list of all user credentials
#[tracing::instrument]
pub async fn users() -> crate::Result<Vec<Credentials>> {
    let state = State::get().await?;
    let users = Credentials::get_all(&state.pool).await?;
    Ok(users.into_iter().map(|x| x.1).collect())
}
