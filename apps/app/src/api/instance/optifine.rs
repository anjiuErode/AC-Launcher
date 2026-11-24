use crate::api::Result;
use anyhow::Context;
use std::path::PathBuf;
use tauri::{Runtime, WebviewWindow};
use tauri_plugin_dialog::DialogExt;
use theseus::instance::{self, OptiFineState};

#[tauri::command]
pub async fn instance_optifine_status(instance_id: &str) -> Result<OptiFineState> {
    Ok(instance::optifine_status(instance_id).await?)
}

#[tauri::command]
pub async fn instance_install_optifine<R: Runtime>(
    window: WebviewWindow<R>,
    instance_id: &str,
) -> Result<Option<OptiFineState>> {
    let (send, receive) = tokio::sync::oneshot::channel();
    window
        .dialog()
        .file()
        .add_filter("OptiFine installer", &["jar"])
        .pick_file(|path| {
            let _ = send.send(path);
        });
    let Some(path) = receive.await? else {
        return Ok(None);
    };
    let path = PathBuf::try_from(path).context("invalid OptiFine installer path")?;
    Ok(Some(instance::install_optifine(instance_id, &path).await?))
}
