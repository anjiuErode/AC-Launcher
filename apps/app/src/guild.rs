//! 公会账号：通过官网授权页登录（浏览器 + 本机回环回调）
//!
//! 身份来自官网「功能页账号」，每个成员账号独立，令牌通过 `x-studio-token` 头调用官网接口。
//!
//! 流程（RFC 8252 原生应用标准做法）：
//! 1. 启动器在 127.0.0.1 上监听随机端口
//! 2. 打开浏览器到 `https://yingzhou1110.top/acl/auth?port=<port>&state=<nonce>&v=1`
//! 3. 用户在页面上登录 / 确认授权
//! 4. 页面跳回 `http://127.0.0.1:<port>/callback?state=...&token=...&user_id=...&name=...`
//! 5. 启动器校验 state，保存会话，页面提示可关闭
//!
//! 令牌不进入 webview，前端只拿得到显示名与用户 ID。

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// 官网授权页地址
const AUTH_PAGE: &str = "https://yingzhou1110.top/acl/auth";
/// 等待用户完成授权的上限
const LOGIN_TIMEOUT_SECS: u64 = 300;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GuildSession {
	/// 官网签发的会话令牌，调用接口时放在 x-studio-token 头
	pub token: String,
	pub user_id: String,
	pub name: Option<String>,
}

/// 返回给前端的会话视图，不包含令牌
#[derive(Clone, Debug, Serialize)]
pub struct PublicSession {
	pub user_id: String,
	pub name: Option<String>,
}

impl From<&GuildSession> for PublicSession {
	fn from(session: &GuildSession) -> Self {
		Self {
			user_id: session.user_id.clone(),
			name: session.name.clone(),
		}
	}
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

/// 简易百分号解码，够用于 display name 这类字段
fn percent_decode(input: &str) -> String {
	let bytes = input.as_bytes();
	let mut out = Vec::with_capacity(bytes.len());
	let mut i = 0;
	while i < bytes.len() {
		match bytes[i] {
			b'%' if i + 2 < bytes.len() => {
				let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("");
				if let Ok(value) = u8::from_str_radix(hex, 16) {
					out.push(value);
					i += 3;
				} else {
					out.push(bytes[i]);
					i += 1;
				}
			}
			b'+' => {
				out.push(b' ');
				i += 1;
			}
			byte => {
				out.push(byte);
				i += 1;
			}
		}
	}
	String::from_utf8_lossy(&out).into_owned()
}

fn query_param(query: &str, key: &str) -> Option<String> {
	query.split('&').find_map(|pair| {
		let (name, value) = pair.split_once('=')?;
		if name == key {
			Some(percent_decode(value))
		} else {
			None
		}
	})
}

fn callback_page(title: &str, detail: &str, ok: bool) -> String {
	let accent = if ok { "#16a34a" } else { "#dc2626" };
	format!(
		r#"<!doctype html><html lang="zh-CN"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1"><title>{title}</title>
<style>
body{{margin:0;min-height:100vh;display:flex;align-items:center;justify-content:center;
background:#0b0b0c;color:#f5f5f5;font-family:system-ui,-apple-system,"Segoe UI",sans-serif}}
.card{{max-width:26rem;padding:2.5rem;border-radius:1rem;background:#161618;text-align:center;border:1px solid #26262a}}
h1{{margin:0 0 .75rem;font-size:1.35rem;color:{accent}}}
p{{margin:0;line-height:1.6;color:#a1a1aa;font-size:.95rem}}
</style></head><body><div class="card"><h1>{title}</h1><p>{detail}</p></div></body></html>"#
	)
}

async fn respond(stream: &mut tokio::net::TcpStream, body: String) {
	let response = format!(
		"HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
		body.as_bytes().len(),
		body
	);
	let _ = stream.write_all(response.as_bytes()).await;
	let _ = stream.flush().await;
}

/// 打开官网授权页并等待回调，成功后保存会话
pub async fn begin_login(identifier: &str) -> Result<PublicSession, String> {
	let listener = TcpListener::bind("127.0.0.1:0")
		.await
		.map_err(|err| format!("无法监听本机端口：{err}"))?;
	let port = listener
		.local_addr()
		.map_err(|err| err.to_string())?
		.port();

	let state: String = {
		use rand::Rng;
		let mut rng = rand::thread_rng();
		(0..32)
			.map(|_| {
				let idx: usize = rng.gen_range(0..16);
				std::char::from_digit(idx as u32, 16).unwrap_or('0')
			})
			.collect()
	};

	let url = format!("{AUTH_PAGE}?port={port}&state={state}&v=1");
	if let Err(err) = tauri_plugin_opener::open_url(&url, None::<&str>) {
		return Err(format!("无法打开浏览器：{err}"));
	}

	let session = match tokio::time::timeout(
		std::time::Duration::from_secs(LOGIN_TIMEOUT_SECS),
		accept_loop(listener, &state),
	)
	.await
	{
		Ok(Ok(session)) => session,
		Ok(Err(err)) => return Err(err),
		Err(_) => return Err("登录等待超时，请重新发起登录".to_string()),
	};

	write_session(identifier, &session)?;
	Ok(PublicSession::from(&session))
}

async fn accept_loop(listener: TcpListener, expected_state: &str) -> Result<GuildSession, String> {
	loop {
		let (mut stream, _) = listener
			.accept()
			.await
			.map_err(|err| format!("接收回调失败：{err}"))?;

		let mut buffer = vec![0u8; 8192];
		let read = match stream.read(&mut buffer).await {
			Ok(n) => n,
			Err(_) => continue,
		};
		let request = String::from_utf8_lossy(&buffer[..read]);
		let target = request
			.lines()
			.next()
			.unwrap_or_default()
			.split_whitespace()
			.nth(1)
			.unwrap_or("/")
			.to_string();

		let (path, query) = match target.split_once('?') {
			Some((path, query)) => (path, query),
			None => (target.as_str(), ""),
		};

		if path != "/callback" {
			respond(
				&mut stream,
				callback_page("无效请求", "这个页面只用于接收 ACL 的登录回调。", false),
			)
			.await;
			continue;
		}

		if query_param(query, "state").unwrap_or_default() != expected_state {
			respond(
				&mut stream,
				callback_page("校验失败", "登录校验码不匹配，请回到启动器重新登录。", false),
			)
			.await;
			return Err("登录校验失败，请重试".to_string());
		}

		if let Some(error) = query_param(query, "error") {
			respond(
				&mut stream,
				callback_page("已取消", "你取消了本次授权，可以回到启动器重试。", false),
			)
			.await;
			return Err(format!("授权已取消（{error}）"));
		}

		let token = query_param(query, "token").unwrap_or_default();
		let user_id = query_param(query, "user_id").unwrap_or_default();
		if token.is_empty() || user_id.is_empty() {
			respond(
				&mut stream,
				callback_page("缺少信息", "授权结果不完整，请回到启动器重新登录。", false),
			)
			.await;
			return Err("授权结果不完整，请重试".to_string());
		}

		let session = GuildSession {
			token,
			user_id,
			name: query_param(query, "name").filter(|value| !value.is_empty()),
		};

		respond(
			&mut stream,
			callback_page("登录成功", "可以关闭此页面，回到 ACL 继续使用。", true),
		)
		.await;
		return Ok(session);
	}
}

/// 当前会话；令牌有效性由官网接口判定，这里只在本地缺失时返回 None
pub fn current(identifier: &str) -> Option<PublicSession> {
	read_session(identifier).map(|session| PublicSession::from(&session))
}

pub fn sign_out(identifier: &str) -> Result<(), String> {
	clear_session(identifier)
}
