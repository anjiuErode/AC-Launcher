use crate::launcher::GameLaunchIdentity;
use crate::{ErrorKind, Result, State};
use anyhow::{Context, bail, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use reqwest::{Client, Response, StatusCode, header};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::{Executor, Sqlite, SqliteConnection};
use std::collections::HashMap;
use std::sync::LazyLock;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
use url::Url;
use uuid::Uuid;

const ACCOUNT_PREFIX: &str = "minecraft_external_account:";
const JSON_LIMIT: usize = 128 * 1024;
const CHOICE_EXPIRY: Duration = Duration::from_secs(300);
const ALI_HEADER: &str = "x-authlib-injector-api-location";
static PENDING: LazyLock<Mutex<HashMap<Uuid, PendingLogin>>> =
	LazyLock::new(|| Mutex::new(HashMap::new()));
static RENEW: Mutex<()> = Mutex::const_new(());
#[cfg(not(windows))]
static SESSION_TOKENS: LazyLock<Mutex<HashMap<Uuid, Tokens>>> =
	LazyLock::new(|| Mutex::new(HashMap::new()));

#[derive(Clone, Serialize, Deserialize)]
pub struct ExternalAuthServer {
	pub api_root: Url,
	pub name: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct ExternalProfile {
	pub id: Uuid,
	pub name: String,
}

#[derive(Serialize)]
pub struct ExternalAccount {
	pub kind: &'static str,
	pub profile: ExternalProfile,
	pub server: ExternalAuthServer,
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ExternalLogin {
	Ready,
	ChooseProfile { login_id: Uuid, profiles: Vec<ExternalProfile> },
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Tokens {
	access_token: String,
	client_token: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AuthResponse {
	#[serde(flatten)]
	tokens: Tokens,
	selected_profile: Option<ExternalProfile>,
	#[serde(default)]
	available_profiles: Vec<ExternalProfile>,
}

struct PendingLogin {
	created: Instant,
	server: ExternalAuthServer,
	identifier_hash: String,
	tokens: Tokens,
	profiles: Vec<ExternalProfile>,
}

#[derive(Serialize, Deserialize)]
struct StoredAccount {
	id: Uuid,
	server: ExternalAuthServer,
	profile: ExternalProfile,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	protected_session: Option<String>,
}

impl StoredAccount {
	fn public(&self) -> ExternalAccount {
		ExternalAccount {
			kind: "external",
			profile: ExternalProfile { id: self.id, name: self.profile.name.clone() },
			server: self.server.clone(),
		}
	}
}

fn auth_client() -> Result<Client> {
	Ok(Client::builder()
		.https_only(true)
		.redirect(reqwest::redirect::Policy::none())
		.connect_timeout(Duration::from_secs(10))
		.timeout(Duration::from_secs(30))
		.user_agent(crate::launcher_user_agent())
		.build()?)
}

fn checked_url(url: Url) -> Result<Url> {
	ensure!(
		url.scheme() == "https" && url.host().is_some()
			&& url.username().is_empty() && url.password().is_none()
			&& url.query().is_none() && url.fragment().is_none()
			&& url.as_str().len() <= 2048,
		ErrorKind::InputError("enter an HTTPS authentication server address without credentials, a query, or a fragment".into())
	);
	Ok(url)
}

fn parse_url(input: &str) -> Result<Url> {
	let input = input.trim();
	let url = if input.contains("://") {
		Url::parse(input)
	} else {
		Url::parse(&format!("https://{input}"))
	}.context("invalid external authentication server address")?;
	checked_url(url)
}

fn auth_endpoint(root: &Url, method: &str) -> Result<Url> {
	let suffix = if root.path().ends_with('/') { "" } else { "/" };
	Ok(Url::parse(&format!("{root}{suffix}authserver/{method}"))?)
}

async fn bounded_body(mut response: Response, limit: usize) -> Result<Vec<u8>> {
	if let Some(length) = response.content_length() {
		ensure!(length <= limit as u64, "external authentication response is too large");
	}
	let mut body = Vec::new();
	while let Some(chunk) = response.chunk().await.context("could not read external authentication response")? {
		ensure!(chunk.len() <= limit.saturating_sub(body.len()), "external authentication response is too large");
		body.extend_from_slice(&chunk);
	}
	Ok(body)
}

fn require_success(response: &Response) -> Result<()> {
	if !response.status().is_success() {
		bail!(ErrorKind::InputError(format!(
			"external authentication server declined the request (HTTP {})",
			response.status().as_u16()
		)));
	}
	Ok(())
}

fn metadata_name(metadata: &[u8]) -> Result<Option<String>> {
	let metadata: Value = serde_json::from_slice(metadata)
		.map_err(|_| ErrorKind::InputError("invalid external authentication metadata".into()))?;
	ensure!(
		metadata.get("meta").is_some_and(Value::is_object)
			|| metadata.get("skinDomains").is_some_and(Value::is_array)
			|| metadata.get("signaturePublickey").is_some_and(Value::is_string),
		"the address does not provide external authentication metadata"
	);
	Ok(metadata.get("meta").and_then(|meta| meta.get("serverName")).and_then(Value::as_str)
		.map(|name| name.chars().filter(|c| !c.is_control()).take(100).collect()))
}

async fn metadata_at(client: &Client, root: &Url) -> Result<Vec<u8>> {
	let response = client.get(root.clone()).send().await
		.map_err(|_| ErrorKind::InputError("could not connect to the HTTPS authentication server".into()))?;
	require_success(&response)?;
	let body = bounded_body(response, JSON_LIMIT).await?;
	metadata_name(&body)?;
	Ok(body)
}

pub async fn discover_external_server(address: &str) -> Result<ExternalAuthServer> {
	let client = auth_client()?;
	let mut root = parse_url(address)?;
	let mut followed_ali = false;
	for _ in 0..6 {
		let response = client.get(root.clone()).send().await
			.map_err(|_| ErrorKind::InputError("could not connect to the HTTPS authentication server".into()))?;
		if response.status().is_redirection() {
			let location = response.headers().get(header::LOCATION)
				.context("external authentication redirect is missing its address")?.to_str()?;
			root = checked_url(root.join(location)?)?;
			continue;
		}
		if !followed_ali && let Some(location) = response.headers().get(ALI_HEADER) {
			followed_ali = true;
			let target = checked_url(root.join(location.to_str()?)?)?;
			if target != root {
				root = target;
				continue;
			}
		}
		require_success(&response)?;
		let body = bounded_body(response, JSON_LIMIT).await?;
		return Ok(ExternalAuthServer { api_root: root, name: metadata_name(&body)? });
	}
	bail!(ErrorKind::InputError("too many external authentication server redirects".into()))
}

async fn post(client: &Client, server: &Url, method: &str, body: Value) -> Result<Response> {
	client.post(auth_endpoint(server, method)?).json(&body).send().await
		.map_err(|_| ErrorKind::InputError("could not connect to the HTTPS authentication server".into()).into())
}

async fn read_auth(response: Response, client_token: &str) -> Result<AuthResponse> {
	require_success(&response)?;
	let body = bounded_body(response, JSON_LIMIT).await?;
	let response: AuthResponse = serde_json::from_slice(&body)
		.map_err(|_| ErrorKind::InputError("invalid response from the authentication server".into()))?;
	ensure!(response.tokens.client_token == client_token, "authentication server returned a different client token");
	ensure!(
		!response.tokens.access_token.is_empty() && response.tokens.access_token.len() <= 8192
			&& !response.tokens.access_token.chars().any(|c| c.is_control() || c.is_whitespace()),
		"authentication server supplied an invalid access token"
	);
	Ok(response)
}

fn validate_profile(profile: &ExternalProfile) -> Result<()> {
	ensure!(
		!profile.id.is_nil() && !profile.name.trim().is_empty() && profile.name.len() <= 128
			&& !profile.name.chars().any(char::is_control),
		"authentication server supplied an invalid player profile"
	);
	Ok(())
}

fn account_id(server: &Url, identifier_hash: &str, profile: Uuid) -> Uuid {
	let mut hash = Sha256::new();
	hash.update(server.as_str().trim_end_matches('/'));
	hash.update([0]);
	hash.update(identifier_hash);
	hash.update([0]);
	hash.update(profile.as_bytes());
	let digest = hash.finalize();
	let mut id = [0u8; 16];
	id.copy_from_slice(&digest[..16]);
	id[6] = (id[6] & 0x0f) | 0x80;
	id[8] = (id[8] & 0x3f) | 0x80;
	Uuid::from_bytes(id)
}

pub async fn begin_external_login(
	api_root: &str,
	username: &str,
	password: &str,
) -> Result<ExternalLogin> {
	let username = username.trim();
	ensure!(
		!username.is_empty() && username.len() <= 1024
			&& !password.is_empty() && password.len() <= 4096,
		"enter the external account and password"
	);
	let client = auth_client()?;
	let api_root = parse_url(api_root)?;
	let name = metadata_name(&metadata_at(&client, &api_root).await?)?;
	let client_token = Uuid::new_v4().simple().to_string();
	let response = post(&client, &api_root, "authenticate", json!({
		"username": username,
		"password": password,
		"clientToken": client_token,
		"requestUser": true,
		"agent": { "name": "Minecraft", "version": 1 },
	})).await?;
	let response = read_auth(response, &client_token).await?;
	let identifier_hash = format!("{:x}", Sha256::digest(username.as_bytes()));
	let server = ExternalAuthServer { api_root, name };
	if let Some(profile) = response.selected_profile {
		save_login(server, &identifier_hash, profile, response.tokens).await?;
		return Ok(ExternalLogin::Ready);
	}
	let profiles = response.available_profiles;
	ensure!(!profiles.is_empty() && profiles.len() <= 100, "external account has no available player profiles");
	for profile in &profiles {
		validate_profile(profile)?;
	}
	let mut pending = PENDING.lock().await;
	pending.retain(|_, login| login.created.elapsed() < CHOICE_EXPIRY);
	ensure!(pending.len() < 8, "close another external login and try again");
	let login_id = Uuid::new_v4();
	pending.insert(login_id, PendingLogin {
		created: Instant::now(),
		server,
		identifier_hash,
		tokens: response.tokens,
		profiles: profiles.clone(),
	});
	tokio::spawn(async move {
		tokio::time::sleep(CHOICE_EXPIRY).await;
		discard_external_login(login_id).await;
	});
	Ok(ExternalLogin::ChooseProfile { login_id, profiles })
}

pub async fn choose_external_profile(login_id: Uuid, profile_id: Uuid) -> Result<()> {
	let pending = PENDING.lock().await.remove(&login_id)
		.filter(|login| login.created.elapsed() < CHOICE_EXPIRY)
		.context("external login expired; sign in again")?;
	let profile = pending.profiles.into_iter().find(|profile| profile.id == profile_id)
		.context("selected external player profile was not offered by this server")?;
	let client = auth_client()?;
	let response = refresh(&client, &pending.server.api_root, &pending.tokens, Some(&profile)).await?;
	let selected = matching_profile(response.selected_profile, profile.id)?;
	save_login(pending.server, &pending.identifier_hash, selected, response.tokens).await
}

pub async fn discard_external_login(login_id: Uuid) {
	PENDING.lock().await.remove(&login_id);
}

async fn refresh(client: &Client, root: &Url, tokens: &Tokens, selected: Option<&ExternalProfile>) -> Result<AuthResponse> {
	let mut body = json!({
		"accessToken": tokens.access_token,
		"clientToken": tokens.client_token,
		"requestUser": true,
	});
	if let Some(profile) = selected {
		body["selectedProfile"] = json!({ "id": profile.id.simple().to_string(), "name": profile.name });
	}
	read_auth(post(client, root, "refresh", body).await?, &tokens.client_token).await
}

fn matching_profile(selected: Option<ExternalProfile>, expected_id: Uuid) -> Result<ExternalProfile> {
	let profile = selected.filter(|profile| profile.id == expected_id)
		.context("authentication response did not match the selected player profile")?;
	validate_profile(&profile)?;
	Ok(profile)
}

async fn save_login(server: ExternalAuthServer, identifier_hash: &str, profile: ExternalProfile, tokens: Tokens) -> Result<()> {
	validate_profile(&profile)?;
	let id = account_id(&server.api_root, identifier_hash, profile.id);
	let account = StoredAccount { id, server, profile, protected_session: None };
	let state = State::get().await?;
	{
		let _renew = RENEW.lock().await;
		let mut tx = state.pool.begin_with("BEGIN IMMEDIATE").await?;
		save_account(account, tokens, &mut tx).await?;
		super::alternate_accounts::set_active(&mut tx, id).await?;
		tx.commit().await?;
	}
	crate::onboarding_checklist::mark_logged_into_minecraft().await;
	Ok(())
}

async fn save_account(
	#[allow(unused_mut)] mut account: StoredAccount,
	tokens: Tokens,
	exec: &mut SqliteConnection,
) -> Result<()> {
	#[cfg(windows)]
	{
		account.protected_session = Some(super::external_secrets::protect(&serde_json::to_vec(&tokens)?)?);
	}
	sqlx::query("INSERT INTO app_settings (key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value")
		.bind(format!("{ACCOUNT_PREFIX}{}", account.id))
		.bind(serde_json::to_string(&account)?)
		.execute(exec)
		.await?;
	#[cfg(not(windows))]
	SESSION_TOKENS.lock().await.insert(account.id, tokens);
	Ok(())
}

async fn tokens_for(account: &StoredAccount) -> Result<Tokens> {
	#[cfg(windows)]
	{
		let protected = account.protected_session.as_deref().context("external login session is missing; sign in again")?;
		serde_json::from_slice(&super::external_secrets::unprotect(protected)
			.map_err(|_| ErrorKind::InputError("external login session could not be read; sign in again".into()))?)
			.context("external login session is invalid; sign in again")
	}
	#[cfg(not(windows))]
	SESSION_TOKENS.lock().await.get(&account.id).cloned()
		.context("external login session expired; sign in again")
}

async fn account(
	exec: impl Executor<'_, Database = Sqlite>,
	id: Uuid,
) -> Result<Option<StoredAccount>> {
	let value: Option<String> = sqlx::query_scalar("SELECT value FROM app_settings WHERE key = ?")
		.bind(format!("{ACCOUNT_PREFIX}{id}"))
		.fetch_optional(exec)
		.await?;
	value.map(|value| serde_json::from_str(&value).context("invalid external account data")).transpose()
}

async fn list_accounts(pool: &sqlx::SqlitePool) -> Result<Vec<StoredAccount>> {
	let values: Vec<String> = sqlx::query_scalar("SELECT value FROM app_settings WHERE key GLOB ?")
		.bind(format!("{ACCOUNT_PREFIX}*"))
		.fetch_all(pool)
		.await?;
	values.into_iter().map(|value| serde_json::from_str(&value).context("invalid external account data")).collect()
}

pub async fn external_accounts() -> Result<Vec<ExternalAccount>> {
	let state = State::get().await?;
	Ok(list_accounts(&state.pool).await?.iter().map(StoredAccount::public).collect())
}

pub(super) async fn select_external_account(exec: &mut SqliteConnection, id: Uuid) -> Result<bool> {
	if account(&mut *exec, id).await?.is_none() {
		return Ok(false);
	}
	super::alternate_accounts::set_active(exec, id).await?;
	Ok(true)
}

pub(super) async fn remove_external_account(exec: &mut SqliteConnection, id: Uuid) -> Result<bool> {
	let removed = sqlx::query("DELETE FROM app_settings WHERE key = ?")
		.bind(format!("{ACCOUNT_PREFIX}{id}"))
		.execute(&mut *exec).await?.rows_affected() != 0;
	if removed {
		#[cfg(not(windows))]
		SESSION_TOKENS.lock().await.remove(&id);
		if super::alternate_accounts::active_alternate_id(&mut *exec).await? == Some(id) {
			super::alternate_accounts::clear_active(exec).await?;
		}
	}
	Ok(removed)
}

pub(crate) async fn external_launch_identity(id: Uuid, state: &State) -> Result<Option<GameLaunchIdentity>> {
	let client = auth_client()?;
	let (account, tokens) = {
		let _renew = RENEW.lock().await;
		let Some(mut account) = account(&state.pool, id).await? else {
			return Ok(None);
		};
		let mut tokens = tokens_for(&account).await?;
		checked_url(account.server.api_root.clone())?;
		let validation = post(&client, &account.server.api_root, "validate", json!({
			"accessToken": tokens.access_token,
			"clientToken": tokens.client_token,
		})).await?;
		match validation.status() {
			StatusCode::NO_CONTENT => {}
			StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN | StatusCode::BAD_REQUEST => {
				let updated = refresh(&client, &account.server.api_root, &tokens, None).await
					.context("external login expired; sign in again")?;
				account.profile = matching_profile(updated.selected_profile, account.profile.id)?;
				tokens = updated.tokens;
				let mut tx = state.pool.begin_with("BEGIN IMMEDIATE").await?;
				if self::account(&mut *tx, id).await?.is_none() {
					return Ok(None);
				}
				save_account(StoredAccount {
					id: account.id,
					server: account.server.clone(),
					profile: account.profile.clone(),
					protected_session: None,
				}, tokens.clone(), &mut tx).await?;
				tx.commit().await?;
			}
			status => bail!(ErrorKind::InputError(format!(
				"could not validate the external account (HTTP {})", status.as_u16()
			))),
		}
		(account, tokens)
	};
	let (jar, metadata) = tokio::try_join!(
		super::authlib_injector::agent_jar(state),
		metadata_at(&client, &account.server.api_root)
	)?;
	Ok(Some(GameLaunchIdentity {
		name: account.profile.name,
		uuid: account.profile.id,
		access_token: tokens.access_token,
		user_type: "mojang",
		authlib_jvm_args: vec![
			format!("-javaagent:{}={}", jar.display(), account.server.api_root),
			format!("-Dauthlibinjector.yggdrasil.prefetched={}", STANDARD.encode(metadata)),
		],
	}))
}

pub(crate) async fn external_logging_tokens(pool: &sqlx::SqlitePool) -> Result<Vec<String>> {
	let mut tokens = Vec::new();
	for account in list_accounts(pool).await? {
		if let Ok(token) = tokens_for(&account).await {
			tokens.push(token.access_token);
		}
	}
	Ok(tokens)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn external_auth_endpoints_are_https_and_keep_the_api_root() -> Result<()> {
		let root = parse_url("auth.example.test/api/yggdrasil")?;
		assert_eq!(auth_endpoint(&root, "authenticate")?.as_str(), "https://auth.example.test/api/yggdrasil/authserver/authenticate");
		for address in ["http://auth.example.test", "https://user:pass@auth.example.test", "https://auth.example.test/?key=value"] {
			assert!(parse_url(address).is_err());
		}
		Ok(())
	}

	#[test]
	fn external_account_identity_is_separate_from_player_uuid() -> Result<()> {
		let player = Uuid::new_v4();
		let root = parse_url("https://auth.example.test")?;
		let other = parse_url("https://other.example.test")?;
		assert_ne!(account_id(&root, "user-a", player), player);
		assert_ne!(account_id(&root, "user-a", player), account_id(&root, "user-b", player));
		assert_ne!(account_id(&root, "user-a", player), account_id(&other, "user-a", player));
		Ok(())
	}

	#[test]
	fn external_profile_selection_matches_server_offer() -> Result<()> {
		let player = Uuid::new_v4();
		let reply: AuthResponse = serde_json::from_value(json!({
			"accessToken": "synthetic-token",
			"clientToken": "synthetic-client",
			"availableProfiles": [{"name":"Sample", "id": player.simple().to_string()}],
		}))?;
		assert!(reply.selected_profile.is_none());
		let selected = reply.available_profiles.into_iter().next().context("test player missing")?;
		assert!(matching_profile(Some(selected.clone()), Uuid::nil()).is_err());
		assert_eq!(matching_profile(Some(selected), player)?.id, player);
		assert_eq!(metadata_name(br#"{"meta":{"serverName":"Synthetic"}}"#)?.as_deref(), Some("Synthetic"));
		assert!(metadata_name(br#"{"unexpected":true}"#).is_err());
		Ok(())
	}
}
