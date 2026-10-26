//! 公会账号：基于 Supabase Auth 的邮箱密码登录
//!
//! 与 Modrinth 账号完全独立。仅用于公会专属功能（公告、整合包、服务器、在线状态）。
//! 令牌存于应用配置目录的 guild_session.json，改动请同步 PRIVACY.md。

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

/// 项目 URL 与 anon key 均为公开信息（受 RLS 保护），可随客户端分发
const SUPABASE_URL: &str = "https://guild.yingzhou1110.top";
const SUPABASE_ANON_KEY: &str =
	"eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJpc3MiOiJzdXBhYmFzZSIsInJlZiI6ImhlaHpmaWFmanF0aGV0ZHhla3hmIiwicm9sZSI6ImFub24iLCJpYXQiOjE3OTMwMDM5NzQsImV4cCI6MjEwODU3OTk3NH0.JDHuCJ89Wm5T-N_MntRIuaogV7sf2qoYl_Rf-BWvDrQ";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GuildSession {
	pub access_token: String,
	pub refresh_token: String,
	pub expires_at: u64,
	pub user_id: String,
	pub email: Option<String>,
}

impl GuildSession {
	fn needs_refresh(&self) -> bool {
		now() + 60 >= self.expires_at
	}
}

/// 返回给前端的会话视图，不包含令牌
#[derive(Clone, Debug, Serialize)]
pub struct PublicSession {
	pub user_id: String,
	pub email: Option<String>,
	pub expires_at: u64,
}

impl From<&GuildSession> for PublicSession {
	fn from(session: &GuildSession) -> Self {
		Self {
			user_id: session.user_id.clone(),
			email: session.email.clone(),
			expires_at: session.expires_at,
		}
	}
}

fn now() -> u64 {
	SystemTime::now()
		.duration_since(UNIX_EPOCH)
		.map(|d| d.as_secs())
		.unwrap_or(0)
}

fn session_path(identifier: &str) -> Option<PathBuf> {
	crate::app_config_dir_for(identifier).map(|dir| dir.join("guild_session.json"))
}

fn read_session(identifier: &str) -> Option<GuildSession> {
	let path = session_path(identifier)?;
	let raw = std::fs::read_to_string(path).ok()?;
	serde_json::from_str(&raw).ok()
}

fn write_session(identifier: &str, session: &GuildSession) -> Result<(), String> {
	let path = session_path(identifier).ok_or_else(|| "无法定位应用配置目录".to_string())?;
	if let Some(parent) = path.parent() {
		std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
	}
	let raw = serde_json::to_string_pretty(session).map_err(|e| e.to_string())?;
	std::fs::write(path, raw).map_err(|e| e.to_string())
}

fn clear_session(identifier: &str) -> Result<(), String> {
	if let Some(path) = session_path(identifier) {
		if path.exists() {
			std::fs::remove_file(path).map_err(|e| e.to_string())?;
		}
	}
	Ok(())
}

#[derive(Deserialize)]
struct AuthUser {
	id: String,
	email: Option<String>,
}

#[derive(Deserialize)]
struct AuthPayload {
	access_token: String,
	refresh_token: String,
	expires_in: u64,
	user: AuthUser,
}

fn parse_auth_error(body: &str) -> String {
	serde_json::from_str::<serde_json::Value>(body)
		.ok()
		.and_then(|value| {
			["error_description", "msg", "error"]
				.iter()
				.find_map(|key| value.get(*key).and_then(|v| v.as_str()).map(String::from))
		})
		.unwrap_or_else(|| "请求失败，请稍后再试".to_string())
}

async fn request_token(
	grant_type: &str,
	body: serde_json::Value,
) -> Result<AuthPayload, String> {
	let url = format!("{SUPABASE_URL}/auth/v1/token?grant_type={grant_type}");
	let response = theseus::REQWEST_CLIENT
		.post(&url)
		.header("apikey", SUPABASE_ANON_KEY)
		.header("Content-Type", "application/json")
		.json(&body)
		.send()
		.await
		.map_err(|err| format!("网络错误：{err}"))?;

	let status = response.status();
	let text = response.text().await.map_err(|err| err.to_string())?;
	if !status.is_success() {
		return Err(parse_auth_error(&text));
	}
	serde_json::from_str::<AuthPayload>(&text)
		.map_err(|err| format!("响应解析失败：{err}"))
}

fn to_session(payload: AuthPayload) -> GuildSession {
	GuildSession {
		expires_at: now() + payload.expires_in,
		access_token: payload.access_token,
		refresh_token: payload.refresh_token,
		user_id: payload.user.id,
		email: payload.user.email,
	}
}

pub async fn sign_in(
	identifier: &str,
	email: &str,
	password: &str,
) -> Result<PublicSession, String> {
	let payload = request_token(
		"password",
		serde_json::json!({ "email": email.trim(), "password": password }),
	)
	.await?;

	let session = to_session(payload);
	write_session(identifier, &session)?;
	Ok(PublicSession::from(&session))
}

/// 注册。若项目开启了邮箱确认，返回提示而不会建立会话。
pub async fn sign_up(identifier: &str, email: &str, password: &str) -> Result<String, String> {
	let url = format!("{SUPABASE_URL}/auth/v1/signup");
	let response = theseus::REQWEST_CLIENT
		.post(&url)
		.header("apikey", SUPABASE_ANON_KEY)
		.header("Content-Type", "application/json")
		.json(&serde_json::json!({ "email": email.trim(), "password": password }))
		.send()
		.await
		.map_err(|err| format!("网络错误：{err}"))?;

	let status = response.status();
	let text = response.text().await.map_err(|err| err.to_string())?;
	if !status.is_success() {
		return Err(parse_auth_error(&text));
	}

	if serde_json::from_str::<AuthPayload>(&text).is_ok() {
		let payload: AuthPayload = serde_json::from_str(&text).map_err(|e| e.to_string())?;
		let session = to_session(payload);
		write_session(identifier, &session)?;
		Ok("注册成功，已自动登录".to_string())
	} else {
		Ok("注册成功，请到邮箱点击确认链接后再登录".to_string())
	}
}

/// 读取会话；过期时自动刷新。刷新失败则清除本地会话。
pub async fn current(identifier: &str) -> Option<PublicSession> {
	let session = read_session(identifier)?;
	if !session.needs_refresh() {
		return Some(PublicSession::from(&session));
	}

	match request_token(
		"refresh_token",
		serde_json::json!({ "refresh_token": session.refresh_token }),
	)
	.await
	{
		Ok(payload) => {
			let refreshed = to_session(payload);
			if write_session(identifier, &refreshed).is_ok() {
				Some(PublicSession::from(&refreshed))
			} else {
				Some(PublicSession::from(&session))
			}
		}
		Err(_) => {
			let _ = clear_session(identifier);
			None
		}
	}
}

pub fn sign_out(identifier: &str) -> Result<(), String> {
	clear_session(identifier)
}
