use crate::State;
use crate::state::MinecraftProfile;
use anyhow::{Context, ensure};
use md5::{Digest, Md5};
use serde::Serialize;
use sqlx::{Executor, Sqlite, SqliteConnection};
use uuid::Uuid;

const ACTIVE_ACCOUNT: &str = "minecraft_active_alternate_account";
const OFFLINE_PREFIX: &str = "minecraft_offline_account:";

#[derive(Serialize)]
pub struct OfflineAccount {
    pub profile: MinecraftProfile,
    pub kind: &'static str,
}

pub(crate) async fn active_alternate_id(
    exec: impl Executor<'_, Database = Sqlite>,
) -> crate::Result<Option<Uuid>> {
    let value: Option<String> =
        sqlx::query_scalar("SELECT value FROM app_settings WHERE key = ?")
            .bind(ACTIVE_ACCOUNT)
            .fetch_optional(exec)
            .await?;
    value
        .map(|value| Uuid::parse_str(&value).context("invalid selected game account"))
        .transpose()
}

pub(super) async fn set_active(exec: &mut SqliteConnection, id: Uuid) -> crate::Result<()> {
    sqlx::query(
        "INSERT INTO app_settings (key, value) VALUES (?, ?)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )
    .bind(ACTIVE_ACCOUNT)
    .bind(id.to_string())
    .execute(exec)
    .await?;
    Ok(())
}

pub(super) async fn clear_active(exec: &mut SqliteConnection) -> crate::Result<()> {
    sqlx::query("DELETE FROM app_settings WHERE key = ?")
        .bind(ACTIVE_ACCOUNT)
        .execute(exec)
        .await?;
    Ok(())
}

fn offline_profile(name: String) -> crate::Result<MinecraftProfile> {
    ensure!(
        (3..=16).contains(&name.len()) &&
            name.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'_'),
        "a Minecraft player name must contain 3–16 letters, digits, or underscores",
    );
    let digest = Md5::digest(format!("OfflinePlayer:{name}"));
    let id = uuid::Builder::from_md5_bytes(digest.into()).into_uuid();
    Ok(MinecraftProfile {
        id,
        name,
        ..MinecraftProfile::default()
    })
}

pub async fn add_offline_account(name: &str) -> crate::Result<OfflineAccount> {
    let profile = offline_profile(name.trim().to_string())?;
    let state = State::get().await?;
    let mut tx = state.pool.begin_with("BEGIN IMMEDIATE").await?;
    sqlx::query(
        "INSERT INTO app_settings (key, value) VALUES (?, ?) ON CONFLICT(key) DO NOTHING",
    )
    .bind(format!("{OFFLINE_PREFIX}{}", profile.id))
    .bind(&profile.name)
    .execute(&mut *tx)
    .await?;
    set_active(&mut tx, profile.id).await?;
    tx.commit().await?;
    crate::onboarding_checklist::mark_logged_into_minecraft().await;
    Ok(OfflineAccount {
        profile,
        kind: "offline",
    })
}

pub async fn offline_accounts() -> crate::Result<Vec<OfflineAccount>> {
    let state = State::get().await?;
    let names = sqlx::query_scalar(
        "SELECT value FROM app_settings WHERE key GLOB ? ORDER BY value COLLATE NOCASE",
    )
    .bind(format!("{OFFLINE_PREFIX}*"))
    .fetch_all(&state.pool)
    .await?;
    names
        .into_iter()
        .map(|name| {
            Ok(OfflineAccount {
                profile: offline_profile(name)?,
                kind: "offline",
            })
        })
        .collect()
}

pub(crate) async fn offline_account(
    exec: impl Executor<'_, Database = Sqlite>,
    id: Uuid,
) -> crate::Result<Option<MinecraftProfile>> {
    let name = sqlx::query_scalar("SELECT value FROM app_settings WHERE key = ?")
        .bind(format!("{OFFLINE_PREFIX}{id}"))
        .fetch_optional(exec)
        .await?;
    name.map(offline_profile).transpose()
}

pub(super) async fn select_offline_account(
    exec: &mut SqliteConnection,
    id: Uuid,
) -> crate::Result<bool> {
    if offline_account(&mut *exec, id).await?.is_none() {
        return Ok(false);
    }
    set_active(exec, id).await?;
    Ok(true)
}

pub(super) async fn remove_offline_account(
    exec: &mut SqliteConnection,
    id: Uuid,
) -> crate::Result<bool> {
    let removed = sqlx::query("DELETE FROM app_settings WHERE key = ?")
        .bind(format!("{OFFLINE_PREFIX}{id}"))
        .execute(&mut *exec)
        .await?
        .rows_affected() != 0;
    if removed && active_alternate_id(&mut *exec).await? == Some(id) {
        clear_active(exec).await?;
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::{
        OFFLINE_PREFIX, active_alternate_id, offline_profile, remove_offline_account,
        select_offline_account,
    };

    #[test]
    fn derives_the_java_offline_uuid_and_rejects_invalid_player_names() {
        let result = offline_profile("Notch".to_string()).unwrap();
        assert_eq!(result.id.to_string(), "b50ad385-829d-3141-a216-7e7d7539ba7f");
        assert!(offline_profile("bad name".to_string()).is_err());
        assert!(offline_profile("玩家".to_string()).is_err());
        assert_ne!(result.id, offline_profile("notch".to_string()).unwrap().id);
    }

    #[tokio::test]
    async fn selecting_and_removing_an_offline_profile_updates_the_stored_default() {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
        sqlx::query("CREATE TABLE app_settings (key TEXT PRIMARY KEY, value TEXT NOT NULL)")
            .execute(&pool)
            .await
            .unwrap();
        let profile = offline_profile("LocalTester".into()).unwrap();
        sqlx::query("INSERT INTO app_settings (key, value) VALUES (?, ?)")
            .bind(format!("{OFFLINE_PREFIX}{}", profile.id))
            .bind(&profile.name)
            .execute(&pool)
            .await
            .unwrap();
        let mut conn = pool.acquire().await.unwrap();
        assert!(select_offline_account(&mut conn, profile.id).await.unwrap());
        assert_eq!(active_alternate_id(&mut *conn).await.unwrap(), Some(profile.id));
        assert!(remove_offline_account(&mut conn, profile.id).await.unwrap());
        assert_eq!(active_alternate_id(&mut *conn).await.unwrap(), None);
    }
}
