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
//!
//! 本模块同时承担 `/api/guild/*` 数据接口的调用：同样在 Rust 侧发起，
//! 界面只拿到解析后的数据，令牌不经过界面层。

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::{LazyLock, Mutex};
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

// ============ 公会数据接口（/api/guild/*） ============
//
// 令牌不下发到 webview：所有数据请求都在 Rust 侧发起，前端只拿到解析后的数据。
// 契约：`docs-acl/guild-api-contract.md` v1；服务端实现见官网 `functions/api/_guild.js`。

/// 官网根地址（与授权页同源）
const API_BASE: &str = "https://yingzhou1110.top";
/// 单次请求上限，避免代理异常时界面无限转圈
const API_TIMEOUT_SECS: u64 = 15;

/// 前端可判别的错误。`code` 决定界面提示「重新登录」还是「数据暂时不可用」。
#[derive(Debug, Serialize)]
pub struct GuildApiError {
	pub code: &'static str,
	pub message: String,
}

fn api_error(code: &'static str, message: impl Into<String>) -> GuildApiError {
	GuildApiError {
		code,
		message: message.into(),
	}
}

/// 服务端形如 `{ "items": [...] }` 的列表响应
#[derive(Debug, Deserialize)]
struct Items<T> {
	items: Vec<T>,
}

/// `GET /api/guild/me`
#[derive(Debug, Deserialize, Serialize)]
pub struct GuildMe {
	/// studio_users.id（UUID 文本，契约示例写的是整数，以服务端实际值为准）
	pub user_id: String,
	pub username: String,
	/// `user` / `admin`
	pub role: String,
	pub joined_at: Option<String>,
}

/// 公告附件（元数据；文件内容走官网公开下载地址，不需要令牌）
#[derive(Debug, Deserialize, Serialize)]
pub struct GuildAttachment {
	pub id: serde_json::Value,
	pub name: String,
	pub size: u64,
	pub mime: String,
}

/// 公告。数据源是官网 `news` 表，字段语义随站内新闻保持一致。
#[derive(Debug, Deserialize, Serialize)]
pub struct GuildAnnouncement {
	/// 类型随数据源（news 表 id 可能是数字或文本），界面只当列表标识用
	pub id: serde_json::Value,
	pub title: String,
	pub body: String,
	pub pinned: bool,
	pub published_at: Option<String>,
	pub author: String,
	/// 老接口没有这个字段，缺省按无附件处理
	#[serde(default)]
	pub attachments: Vec<GuildAttachment>,
}

/// 整合包。`sha256` 由服务端转大写，缺失时为空串。
#[derive(Debug, Deserialize, Serialize)]
pub struct GuildPack {
	pub id: serde_json::Value,
	pub name: String,
	pub mc_version: String,
	pub loader: String,
	pub description: String,
	pub url: String,
	pub sha256: String,
	pub size_bytes: u64,
}

/// 服务器。服务端已过滤 host 带端口/协议、port 越界的脏行。
#[derive(Debug, Deserialize, Serialize)]
pub struct GuildServer {
	pub id: serde_json::Value,
	pub name: String,
	pub host: String,
	/// 用 u32 而非 u16：脏数据不该让整个列表解析失败
	pub port: u32,
	pub description: String,
	pub whitelist_required: bool,
}

async fn get_json<T: DeserializeOwned>(
	identifier: &str,
	path: &str,
) -> Result<T, GuildApiError> {
	let session = read_session(identifier)
		.ok_or_else(|| api_error("not_signed_in", "尚未登录公会账号"))?;

	let response = theseus::REQWEST_CLIENT
		.get(format!("{API_BASE}{path}"))
		.header("x-studio-token", &session.token)
		.timeout(std::time::Duration::from_secs(API_TIMEOUT_SECS))
		.send()
		.await
		.map_err(|err| api_error("network", format!("无法连接官网：{err}")))?;

	let status = response.status();
	if status.as_u16() == 401 {
		// 契约：401 一律视为会话失效，清掉本地登录态让用户重新登录
		let _ = clear_session(identifier);
		return Err(api_error("unauthorized", "登录已过期，请重新登录"));
	}

	let body = response
		.text()
		.await
		.map_err(|err| api_error("network", format!("读取官网响应失败：{err}")))?;

	if !status.is_success() {
		let message = serde_json::from_str::<serde_json::Value>(&body)
			.ok()
			.and_then(|value| {
				value
					.get("error")
					.and_then(|error| error.as_str())
					.map(str::to_string)
			})
			.unwrap_or_else(|| format!("官网返回 {status}"));
		return Err(api_error("server", message));
	}

	serde_json::from_str(&body)
		.map_err(|err| api_error("bad_response", format!("官网数据格式异常：{err}")))
}

pub async fn fetch_me(identifier: &str) -> Result<GuildMe, GuildApiError> {
	get_json(identifier, "/api/guild/me").await
}

pub async fn fetch_announcements(
	identifier: &str,
	limit: Option<u32>,
) -> Result<Vec<GuildAnnouncement>, GuildApiError> {
	// 与服务端一致：默认 20，上限 50
	let limit = limit.unwrap_or(20).clamp(1, 50);
	let body: Items<GuildAnnouncement> = get_json(
		identifier,
		&format!("/api/guild/announcements?limit={limit}"),
	)
	.await?;
	Ok(body.items)
}

/// 附件文本内容上限：再大就不该在公告卡里内联，交给浏览器打开
const ATTACHMENT_TEXT_LIMIT: usize = 1024 * 1024;

/// 只接受安全字符，避免把参数拼进路径时注入别的地址
fn safe_segment(value: &str) -> Result<&str, GuildApiError> {
	if value.is_empty()
		|| value.len() > 128
		|| !value
			.chars()
			.all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
	{
		return Err(api_error("bad_request", "附件地址不合法"));
	}
	Ok(value)
}

/// 读取附件文本（Markdown / 纯文本），供界面内联预览。
///
/// 附件下载地址是公开的，不需要令牌；这里仍然在 Rust 侧请求，
/// 避免 webview 直连时被 CORS 拦住。
pub async fn fetch_attachment_text(
	news_id: &str,
	attachment_id: &str,
) -> Result<String, GuildApiError> {
	let news_id = safe_segment(news_id)?;
	let attachment_id = safe_segment(attachment_id)?;
	let url = format!(
		"{API_BASE}/api/news/{news_id}/attachments/{attachment_id}?inline=1"
	);

	let response = theseus::REQWEST_CLIENT
		.get(url)
		.timeout(std::time::Duration::from_secs(API_TIMEOUT_SECS))
		.send()
		.await
		.map_err(|err| api_error("network", format!("无法读取附件：{err}")))?;

	let status = response.status();
	if !status.is_success() {
		return Err(api_error(
			"server",
			format!("官网返回 {status}，附件可能已被删除"),
		));
	}

	let bytes = response
		.bytes()
		.await
		.map_err(|err| api_error("network", format!("读取附件内容失败：{err}")))?;
	if bytes.len() > ATTACHMENT_TEXT_LIMIT {
		return Err(api_error("too_large", "附件太大，请用浏览器打开"));
	}
	// 文本附件不保证是合法 UTF-8，坏字节按替换字符显示，总比整块失败好
	Ok(String::from_utf8_lossy(&bytes).into_owned())
}

pub async fn fetch_packs(identifier: &str) -> Result<Vec<GuildPack>, GuildApiError> {
	let body: Items<GuildPack> = get_json(identifier, "/api/guild/packs").await?;
	Ok(body.items)
}

pub async fn fetch_servers(identifier: &str) -> Result<Vec<GuildServer>, GuildApiError> {
	let body: Items<GuildServer> = get_json(identifier, "/api/guild/servers").await?;
	Ok(body.items)
}

// ---------------------------------------------------------------------------
// 在线状态（P3）
//
// 分工：界面负责告诉后台「现在在玩哪个实例」，后台负责每 30 秒上报一次。
// 之所以把心跳放在 Rust：启动器启动游戏后默认最小化到托盘，webview 的
// setInterval 会被节流甚至暂停，前端定时器做心跳必然掉线。
// ---------------------------------------------------------------------------

/// 当前在玩什么。`running=false` 表示没有游戏在跑，此时不上报。
#[derive(Clone, Debug, Default, Serialize)]
pub struct PresenceState {
	pub running: bool,
	pub instance_name: String,
	pub mc_version: String,
	pub loader: String,
}

static PRESENCE_STATE: LazyLock<Mutex<PresenceState>> =
	LazyLock::new(|| Mutex::new(PresenceState::default()));

/// 成员列表里的一个人
#[derive(Debug, Deserialize, Serialize)]
pub struct GuildPresence {
	pub username: String,
	pub instance_name: String,
	pub mc_version: String,
	pub last_seen: String,
}

/// 本地偏好：隐身开关，默认可见
#[derive(Debug, Clone, Serialize, Deserialize)]
struct GuildPrefs {
	#[serde(default = "default_presence_visible")]
	presence_visible: bool,
}

fn default_presence_visible() -> bool {
	true
}

impl Default for GuildPrefs {
	fn default() -> Self {
		Self {
			presence_visible: true,
		}
	}
}

fn prefs_path(identifier: &str) -> Option<PathBuf> {
	crate::app_config_dir_for(identifier).map(|dir| dir.join("guild_prefs.json"))
}

fn read_prefs(identifier: &str) -> GuildPrefs {
	prefs_path(identifier)
		.and_then(|path| std::fs::read_to_string(path).ok())
		.and_then(|raw| serde_json::from_str(&raw).ok())
		.unwrap_or_default()
}

pub fn presence_visible(identifier: &str) -> bool {
	read_prefs(identifier).presence_visible
}

pub fn set_presence_visible(identifier: &str, visible: bool) -> Result<(), String> {
	let path = prefs_path(identifier).ok_or_else(|| "无法定位应用配置目录".to_string())?;
	if let Some(parent) = path.parent() {
		std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
	}
	let prefs = GuildPrefs {
		presence_visible: visible,
	};
	let raw = serde_json::to_string_pretty(&prefs).map_err(|error| error.to_string())?;
	std::fs::write(path, raw).map_err(|error| error.to_string())
}

/// 界面在游戏启停时调用
pub fn update_presence_state(state: PresenceState) {
	if let Ok(mut guard) = PRESENCE_STATE.lock() {
		*guard = state;
	}
}

pub fn presence_snapshot() -> PresenceState {
	PRESENCE_STATE
		.lock()
		.map(|guard| guard.clone())
		.unwrap_or_default()
}

/// 带 JSON 体的 POST。429 是契约里的正常限流，按成功处理。
async fn post_json(
	identifier: &str,
	path: &str,
	body: &serde_json::Value,
) -> Result<(), GuildApiError> {
	let session = read_session(identifier)
		.ok_or_else(|| api_error("not_signed_in", "尚未登录公会账号"))?;

	let response = theseus::REQWEST_CLIENT
		.post(format!("{API_BASE}{path}"))
		.header("x-studio-token", &session.token)
		.json(body)
		.timeout(std::time::Duration::from_secs(API_TIMEOUT_SECS))
		.send()
		.await
		.map_err(|err| api_error("network", format!("无法连接官网：{err}")))?;

	let status = response.status();
	if status.as_u16() == 401 {
		let _ = clear_session(identifier);
		return Err(api_error("unauthorized", "登录已过期，请重新登录"));
	}
	// 客户端每 30 秒一次，服务端 10 秒内重复会回 429 —— 这是正常节奏，不该当错误
	if status.as_u16() == 429 || status.is_success() {
		return Ok(());
	}

	let body = response.text().await.unwrap_or_default();
	let message = serde_json::from_str::<serde_json::Value>(&body)
		.ok()
		.and_then(|value| {
			value
				.get("error")
				.and_then(|error| error.as_str())
				.map(str::to_string)
		})
		.unwrap_or_else(|| format!("官网返回 {status}"));
	Err(api_error("server", message))
}

/// 上报一次在线状态。返回 false 表示当前没有在玩游戏（未发送）。
pub async fn report_presence(identifier: &str) -> Result<bool, GuildApiError> {
	let state = presence_snapshot();
	if !state.running {
		return Ok(false);
	}

	let body = serde_json::json!({
		"instance_name": state.instance_name,
		"mc_version": state.mc_version,
		"loader": state.loader,
		"visible": presence_visible(identifier),
	});
	post_json(identifier, "/api/guild/presence", &body).await?;
	Ok(true)
}

pub async fn fetch_presence_list(identifier: &str) -> Result<Vec<GuildPresence>, GuildApiError> {
	let payload: Items<GuildPresence> = get_json(identifier, "/api/guild/presence").await?;
	Ok(payload.items)
}

// ---------------------------------------------------------------------------
// 公会整合包：下载与校验（P4-2）
//
// 下载放 Rust 侧的理由：需要落盘、需要算 SHA256、需要嗅探包格式，
// 而且界面层不该直接拿外部 URL 去 fetch（CSP 与跨域都不方便）。
// ---------------------------------------------------------------------------

/// 下载并校验后的整合包
#[derive(Debug, Serialize)]
pub struct GuildPackFile {
	pub path: String,
	/// `mrpack`（含 modrinth.index.json）或 `zip`（实例压缩包）
	pub format: String,
	pub size_bytes: u64,
}

fn sanitize_filename(name: &str) -> String {
	let cleaned: String = name
		.chars()
		.map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') { c } else { '_' })
		.collect();
	// 去掉可能的前导点，避免生成隐藏文件或 `..` 之类
	let trimmed = cleaned.trim_start_matches('.').to_string();
	if trimmed.is_empty() {
		"pack.zip".to_string()
	} else {
		trimmed
	}
}

async fn detect_pack_format(path: &std::path::Path) -> String {
	match async_zip::tokio::read::fs::ZipFileReader::new(path).await {
		Ok(zip) => {
			let has_index = zip.file().entries().iter().any(|entry| {
				entry
					.filename()
					.as_str()
					.is_ok_and(|name| name == "modrinth.index.json")
			});
			if has_index { "mrpack".to_string() } else { "zip".to_string() }
		}
		// 打不开就按普通压缩包处理，让安装阶段给出更具体的错误
		Err(_) => "zip".to_string(),
	}
}

/// 下载整合包到临时目录并按 SHA256 校验。
///
/// `sha256` 为空表示服务端未提供校验值（契约允许），此时跳过校验但会在日志里留痕。
pub async fn download_pack(
	url: &str,
	sha256: &str,
	filename: &str,
) -> Result<GuildPackFile, GuildApiError> {
	use sha2::{Digest, Sha256};

	if !url.starts_with("https://") {
		return Err(api_error("bad_request", "整合包地址必须是 https 链接"));
	}

	let dir = std::env::temp_dir().join("acl-guild-packs");
	tokio::fs::create_dir_all(&dir)
		.await
		.map_err(|err| api_error("io", format!("无法创建临时目录：{err}")))?;
	let path = dir.join(sanitize_filename(filename));

	let response = theseus::REQWEST_CLIENT
		.get(url)
		.timeout(std::time::Duration::from_secs(900))
		.send()
		.await
		.map_err(|err| api_error("network", format!("无法下载整合包：{err}")))?;

	if !response.status().is_success() {
		return Err(api_error(
			"server",
			format!("整合包下载失败：官网返回 {}", response.status()),
		));
	}

	let mut file = tokio::fs::File::create(&path)
		.await
		.map_err(|err| api_error("io", format!("无法写入临时文件：{err}")))?;
	let mut hasher = Sha256::new();
	let mut size: u64 = 0;
	let mut response = response;

	loop {
		let chunk = response
			.chunk()
			.await
			.map_err(|err| api_error("network", format!("下载中断：{err}")))?;
		let Some(chunk) = chunk else { break };
		hasher.update(&chunk);
		size += chunk.len() as u64;
		file.write_all(&chunk)
			.await
			.map_err(|err| api_error("io", format!("写入失败：{err}")))?;
	}
	file.flush()
		.await
		.map_err(|err| api_error("io", format!("写入失败：{err}")))?;
	drop(file);

	if !sha256.is_empty() {
		let actual = format!("{:X}", hasher.finalize());
		if !actual.eq_ignore_ascii_case(sha256.trim()) {
			let _ = tokio::fs::remove_file(&path).await;
			return Err(api_error(
				"checksum",
				format!("整合包校验失败（期望 {sha256}，实际 {actual}），已丢弃下载文件"),
			));
		}
	} else {
		tracing::warn!("Guild pack {filename} has no sha256; signature not verified");
	}

	let format = detect_pack_format(&path).await;
	Ok(GuildPackFile {
		path: path.to_string_lossy().to_string(),
		format,
		size_bytes: size,
	})
}

// ---------------------------------------------------------------------------
// 公会整合包：格式识别与解压入库（P4-2 的 zip 分支）
//
// 识别顺序参照 PCL2 的做法：能确定就确定，确定不了交给上层让用户选，
// 绝不猜版本 —— 猜错会导致实例根本起不来。
//   1. modrinth.index.json      → mrpack（交给既有安装流程）
//   2. manifest.json            → CurseForge 导出包，含 minecraft.version 与 modLoaders
//   3. versions/<id>/<id>.json  → 实例归档，id / inheritsFrom / libraries 可推版本与加载器
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Default)]
pub struct PackInspection {
	/// mrpack | curseforge | instance | unknown
	pub format: String,
	pub mc_version: Option<String>,
	pub loader: Option<String>,
	pub loader_version: Option<String>,
	/// 包内实例根的前缀，空串表示根目录即实例根，常见为 ".minecraft/"
	pub root_prefix: String,
	pub entry_count: usize,
}

fn normalize_loader(raw: &str) -> Option<(&'static str, String)> {
	let lower = raw.to_ascii_lowercase();
	for name in ["neoforge", "fabric", "quilt", "forge"] {
		if let Some(rest) = lower.strip_prefix(&format!("{name}-")) {
			return Some((name, rest.to_string()));
		}
	}
	None
}

fn detect_loader_from_libraries(value: &serde_json::Value) -> Option<(&'static str, String)> {
	let libraries = value.get("libraries")?.as_array()?;
	for library in libraries {
		let name = library.get("name")?.as_str()?;
		if let Some((loader, version)) = normalize_loader(name) {
			return Some((loader, version));
		}
	}
	None
}

/// 读取 zip 内某个小文件（用于解析清单），超过 8 MB 直接放弃
async fn read_zip_entry(
	zip: &async_zip::tokio::read::fs::ZipFileReader,
	index: usize,
) -> Option<Vec<u8>> {
	if zip.file().entries()[index].uncompressed_size() > 8 * 1024 * 1024 {
		return None;
	}
	let mut bytes = Vec::new();
	zip.reader_with_entry(index)
		.await
		.ok()?
		.read_to_end_checked(&mut bytes)
		.await
		.ok()?;
	Some(bytes)
}

/// 只探测，不落盘
pub async fn inspect_pack(path: &str) -> Result<PackInspection, GuildApiError> {
	let path_ref = std::path::Path::new(path);
	let zip = async_zip::tokio::read::fs::ZipFileReader::new(path_ref)
		.await
		.map_err(|err| api_error("io", format!("无法读取整合包：{err}")))?;

	let entries = zip.file().entries();
	let mut inspection = PackInspection {
		format: "unknown".to_string(),
		entry_count: entries.len(),
		root_prefix: String::new(),
		..Default::default()
	};

	// 是否被包了一层 .minecraft/
	if entries.iter().any(|entry| {
		entry
			.filename()
			.as_str()
			.is_ok_and(|name| name.starts_with(".minecraft/"))
	}) {
		inspection.root_prefix = ".minecraft/".to_string();
	}

	let mut modrinth_index = None;
	let mut curseforge_manifest = None;
	let mut version_json = None;

	for (index, entry) in entries.iter().enumerate() {
		let Ok(name) = entry.filename().as_str() else { continue };
		match name {
			"modrinth.index.json" => modrinth_index = Some(index),
			"manifest.json" => curseforge_manifest = Some(index),
			other => {
				// versions/<id>/<id>.json
				let parts: Vec<&str> = other.split('/').collect();
				if parts.len() == 3 && parts[0] == "versions" && parts[2].ends_with(".json") {
					let id = parts[1];
					if parts[2] == format!("{id}.json") && version_json.is_none() {
						version_json = Some(index);
					}
				}
			}
		}
	}

	if modrinth_index.is_some() {
		inspection.format = "mrpack".to_string();
		return Ok(inspection);
	}

	if let Some(index) = curseforge_manifest
		&& let Some(bytes) = read_zip_entry(&zip, index).await
		&& let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes)
	{
		inspection.format = "curseforge".to_string();
		if let Some(minecraft) = value.get("minecraft") {
			inspection.mc_version = minecraft
				.get("version")
				.and_then(|v| v.as_str())
				.map(str::to_string);
			if let Some(first) = minecraft
				.get("modLoaders")
				.and_then(|v| v.as_array())
				.and_then(|list| list.first())
				.and_then(|v| v.get("id"))
				.and_then(|v| v.as_str())
				&& let Some((loader, version)) = normalize_loader(first)
			{
				inspection.loader = Some(loader.to_string());
				inspection.loader_version = Some(version);
			}
		}
		return Ok(inspection);
	}

	if let Some(index) = version_json
		&& let Some(bytes) = read_zip_entry(&zip, index).await
		&& let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes)
	{
		inspection.format = "instance".to_string();
		// inheritsFrom 存在时它才是原版版本号，id 往往带加载器后缀
		inspection.mc_version = value
			.get("inheritsFrom")
			.and_then(|v| v.as_str())
			.or_else(|| value.get("id").and_then(|v| v.as_str()))
			.map(str::to_string);
		if let Some((loader, version)) = detect_loader_from_libraries(&value) {
			inspection.loader = Some(loader.to_string());
			inspection.loader_version = Some(version);
		}
		return Ok(inspection);
	}

	Ok(inspection)
}

/// 把整合包内容解压到指定实例目录，返回写入的文件数
pub async fn extract_pack_into_instance(
	path: &str,
	instance_id: &str,
	root_prefix: &str,
) -> Result<usize, GuildApiError> {
	let zip = async_zip::tokio::read::fs::ZipFileReader::new(std::path::Path::new(path))
		.await
		.map_err(|err| api_error("io", format!("无法读取整合包：{err}")))?;

	let target_root = theseus::instance::get_full_path(instance_id)
		.await
		.map_err(|err| api_error("not_found", format!("实例不存在：{err}")))?;

	let mut written = 0usize;
	for index in 0..zip.file().entries().len() {
		let entry = &zip.file().entries()[index];
		if entry.dir().is_ok_and(|dir| dir) {
			continue;
		}
		let Ok(name) = entry.filename().as_str() else { continue };
		let Some(relative) = name.strip_prefix(root_prefix) else { continue };
		if relative.is_empty() {
			continue;
		}
		// 防路径穿越：逐段校验，拒绝 ..、绝对路径与盘符
		let mut target = target_root.clone();
		let mut unsafe_path = false;
		for segment in relative.split('/') {
			if segment.is_empty() || segment == "." || segment == ".." || segment.contains(':') || segment.contains('\\') {
				unsafe_path = true;
				break;
			}
			target.push(segment);
		}
		if unsafe_path {
			tracing::warn!("Skipping unsafe path in pack: {name}");
			continue;
		}

		if let Some(parent) = target.parent() {
			let _ = tokio::fs::create_dir_all(parent).await;
		}
		let mut bytes = Vec::new();
		zip.reader_with_entry(index)
			.await
			.map_err(|err| api_error("io", format!("读取 {name} 失败：{err}")))?
			.read_to_end_checked(&mut bytes)
			.await
			.map_err(|err| api_error("io", format!("解压 {name} 失败：{err}")))?;
		tokio::fs::write(&target, &bytes)
			.await
			.map_err(|err| api_error("io", format!("写入 {name} 失败：{err}")))?;
		written += 1;
	}

	Ok(written)
}
