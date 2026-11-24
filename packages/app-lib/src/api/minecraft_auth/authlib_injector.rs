use crate::{Result, State};
use anyhow::{Context, ensure};
use sha2::{Digest, Sha256};
use std::io::Write;
use std::path::PathBuf;
use std::time::Duration;
use tokio::sync::Mutex;

const VERSION: &str = "1.2.8";
const DOWNLOAD: &str = "https://authlib-injector.yushi.moe/artifact/56/authlib-injector-1.2.8.jar";
const SHA256: &str = "9c7f4343e6c82034958ffb48c14a2cb0c85928be7283103ce17da00c6d5a7b10";
const MAX_BYTES: usize = 2 * 1024 * 1024;
static CACHE_ACCESS: Mutex<()> = Mutex::const_new(());

fn verified(bytes: &[u8]) -> bool {
	format!("{:x}", Sha256::digest(bytes)) == SHA256
}

pub(super) async fn agent_jar(state: &State) -> Result<PathBuf> {
	let _lock = CACHE_ACCESS.lock().await;
	let directory = state.directories.metadata_dir().join("authlib-injector");
	let path = directory.join(format!("authlib-injector-{VERSION}.jar"));
	match tokio::fs::symlink_metadata(&path).await {
		Ok(metadata) => {
			ensure!(metadata.is_file() && !metadata.file_type().is_symlink(), "Authlib-Injector cache is not a regular file");
			if metadata.len() <= MAX_BYTES as u64 && verified(&tokio::fs::read(&path).await?) {
				return Ok(path);
			}
		}
		Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
		Err(error) => return Err(error.into()),
	}
	let client = reqwest::Client::builder()
		.https_only(true)
		.redirect(reqwest::redirect::Policy::none())
		.connect_timeout(Duration::from_secs(15))
		.timeout(Duration::from_secs(60))
		.user_agent(crate::launcher_user_agent())
		.build()?;
	let mut response = client.get(DOWNLOAD).send().await.context("could not download official Authlib-Injector")?;
	ensure!(response.status().is_success(), "official Authlib-Injector download returned HTTP {}", response.status().as_u16());
	let mut bytes = Vec::new();
	while let Some(chunk) = response.chunk().await? {
		ensure!(chunk.len() <= MAX_BYTES.saturating_sub(bytes.len()), "Authlib-Injector download is too large");
		bytes.extend_from_slice(&chunk);
	}
	ensure!(verified(&bytes), "official Authlib-Injector failed SHA-256 verification");
	tokio::fs::create_dir_all(&directory).await?;
	let output = path.clone();
	tokio::task::spawn_blocking(move || {
		let mut temp = tempfile::NamedTempFile::new_in(directory)?;
		temp.write_all(&bytes)?;
		temp.as_file().sync_all()?;
		temp.persist(output).map_err(|error| error.error)?;
		Ok::<(), crate::Error>(())
	}).await??;
	Ok(path)
}
