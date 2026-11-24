use crate::api::Result;
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use futures_util::StreamExt;
use minisign_verify::{PublicKey, Signature};
use std::sync::{Arc, Mutex};
use tauri::http::HeaderValue;
use tauri::http::header::ACCEPT;
use tauri::{Manager, ResourceId, Runtime, Webview};
use tauri_plugin_http::reqwest;
use tauri_plugin_updater::{Config, Error, Update};
use theseus::{
    LoadingBarType, ProxyConfig, ProxyMode, emit_loading, init_loading,
    launcher_user_agent,
};
use tokio::time::Instant;

#[derive(Default)]
pub struct PendingUpdateData(pub Mutex<Option<(Arc<Update>, Vec<u8>)>>);

fn update_request(
    update: &Update,
    proxy_config: &ProxyConfig,
    method: reqwest::Method,
) -> std::result::Result<reqwest::RequestBuilder, reqwest::Error> {
    let mut headers = update.headers.clone();
    if !headers.contains_key(ACCEPT) {
        headers.insert(
            ACCEPT,
            HeaderValue::from_static("application/octet-stream"),
        );
    }

    let mut request = if update.no_proxy || update.proxy.is_some() {
        reqwest::Client::builder().user_agent(launcher_user_agent())
    } else {
        theseus::client_builder_with(proxy_config)
    };
    if let Some(timeout) = update.timeout {
        request = request.timeout(timeout);
    }
    if update.no_proxy {
        request = request.no_proxy();
    } else if let Some(ref proxy) = update.proxy {
        let proxy = reqwest::Proxy::all(proxy.as_str())?;
        request = request.proxy(proxy);
    }
    if method == reqwest::Method::GET {
        request = request.https_only(true);
    }
    Ok(request
        .build()?
        .request(method, update.download_url.clone())
        .headers(headers))
}

fn update_http_error(error: reqwest::Error) -> Error {
    if error.is_timeout() {
        Error::Network("Update download request timed out".into())
    } else {
        Error::Network("Update download request failed".into())
    }
}

fn check_update_response(
    response: &reqwest::Response,
) -> std::result::Result<(), Error> {
    if response.status().is_success() {
        Ok(())
    } else {
        Err(Error::Network(format!(
            "Download request failed with status: {}",
            response.status()
        )))
    }
}

#[tauri::command]
pub async fn get_update_size<R: Runtime>(
    webview: Webview<R>,
    rid: ResourceId,
) -> Result<Option<u64>> {
    let update = webview.resources_table().get::<Update>(rid)?;
    let response = update_request(
        &update,
        &theseus::http_proxy_config(),
        reqwest::Method::HEAD,
    )
    .map_err(update_http_error)?
    .send()
    .await
    .map_err(update_http_error)?;
    check_update_response(&response)?;

    let content_length = response
        .headers()
        .get("Content-Length")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse().ok());

    Ok(content_length)
}

fn verify_update_signature(
    data: &[u8],
    encoded_signature: &str,
    encoded_pubkey: &str,
) -> std::result::Result<(), Error> {
    fn decode(encoded: &str) -> Option<String> {
        String::from_utf8(STANDARD.decode(encoded).ok()?).ok()
    }

    let public_key = decode(encoded_pubkey)
        .and_then(|key| PublicKey::decode(&key).ok())
        .ok_or_else(|| {
            Error::Network("Invalid update verification key".into())
        })?;
    let signature = decode(encoded_signature)
        .and_then(|signature| Signature::decode(&signature).ok())
        .ok_or_else(|| Error::Network("Invalid update signature".into()))?;
    public_key
        .verify(data, &signature, true)
        .map_err(|_| Error::Network("Update signature verification failed".into()))
}

async fn try_download_with_shared_client(
    update: &Update,
    config: &Config,
    on_progress: &mut impl FnMut(u64, Option<u64>),
) -> std::result::Result<Option<Vec<u8>>, Error> {
    let proxy = theseus::http_proxy_config();
    let native_route_matches = update.no_proxy
        || update.proxy.is_some()
        || matches!(proxy.mode, ProxyMode::Auto);
    let request = match update_request(update, &proxy, reqwest::Method::GET) {
        Ok(request) => request,
        Err(_) if native_route_matches => return Ok(None),
        Err(error) => return Err(update_http_error(error)),
    };
    let clone_request = || {
        request
            .try_clone()
            .ok_or_else(|| Error::Network("Invalid update download request".into()))
    };
    let response = match clone_request()?.send().await {
        Ok(response) => response,
        Err(error) if error.is_connect() && native_route_matches => {
            return Ok(None);
        }
        Err(error) => return Err(update_http_error(error)),
    };
    check_update_response(&response)?;
    let mut body = theseus::download_body(clone_request()?, response).await;
    let mut bytes = Vec::new();
    while let Some(chunk) = body.stream.next().await {
        match chunk {
            Ok(chunk) => {
                bytes.extend_from_slice(&chunk);
                on_progress(bytes.len() as u64, body.total);
            }
            Err(_) if body.is_segmented() => {
                drop(body);
                let response = clone_request()?
                    .send()
                    .await
                    .map_err(update_http_error)?;
                check_update_response(&response)?;
                body = theseus::DownloadBody::sequential(response);
                bytes.clear();
            }
            Err(_) => {
                return Err(Error::Network("Update download interrupted".into()));
            }
        }
    }
    verify_update_signature(&bytes, &update.signature, &config.pubkey)?;
    Ok(Some(bytes))
}

async fn download_verified_update(
    update: &Update,
    config: Option<Config>,
    mut on_progress: impl FnMut(u64, Option<u64>),
) -> std::result::Result<Vec<u8>, Error> {
    // The plugin's check command has no key override, and our plugin builder
    // sets only User-Agent, so its resource uses this same configured key.
    if let Some(config) = config
        && !config.dangerous_accept_invalid_certs
        && !config.dangerous_accept_invalid_hostnames
        && update.download_url.scheme() == "https"
        && let Some(bytes) =
            try_download_with_shared_client(update, &config, &mut on_progress)
                .await?
    {
        return Ok(bytes);
    }

    let mut downloaded_bytes = 0;
    update
        .download(
            |chunk_size, total_size| {
                downloaded_bytes += chunk_size as u64;
                on_progress(downloaded_bytes, total_size);
            },
            || {},
        )
        .await
        .map_err(|error| match error {
            Error::Network(_) => error,
            Error::Reqwest(error) if error.is_timeout() => {
                Error::Network("Update download request timed out".into())
            }
            Error::Minisign(_) | Error::Base64(_) | Error::SignatureUtf8(_) => {
                Error::Network("Update signature verification failed".into())
            }
            _ => Error::Network("Update download request failed".into()),
        })
}

#[tauri::command]
pub async fn enqueue_update_for_installation<R: Runtime>(
    webview: Webview<R>,
    rid: ResourceId,
) -> Result<()> {
    let pending_data = webview.state::<PendingUpdateData>().inner();

    let update = webview.resources_table().get::<Update>(rid)?;

    let progress = init_loading(
        LoadingBarType::LauncherUpdate {
            version: update.version.clone(),
            current_version: update.current_version.clone(),
        },
        1.0,
        "Downloading update...",
    )
    .await?;

    let download_start = Instant::now();
    let config = webview
        .config()
        .plugins
        .0
        .get("updater")
        .and_then(|config| serde_json::from_value(config.clone()).ok());
    let mut last_fraction = 0.0f64;
    let update_data = download_verified_update(
        &update,
        config,
        |downloaded_bytes, total_size| {
            let Some(total_size) = total_size.filter(|&size| size > 0) else {
                return;
            };
            let fraction = (downloaded_bytes as f64 / total_size as f64).min(0.99);
            if fraction <= last_fraction {
                return;
            }
            if let Err(e) = emit_loading(&progress, fraction - last_fraction, None) {
                tracing::error!("Failed to update download progress bar: {e}");
            }
            last_fraction = fraction;
        },
    )
    .await?;
    let download_duration = download_start.elapsed();
    tracing::info!("Downloaded update in {download_duration:?}");

    pending_data
        .0
        .lock()
        .unwrap()
        .replace((update, update_data));

    Ok(())
}

#[tauri::command]
pub fn remove_enqueued_update<R: Runtime>(webview: Webview<R>) {
    let pending_data = webview.state::<PendingUpdateData>().inner();
    pending_data.0.lock().unwrap().take();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn update_signature_rejects_tampered_payload() {
        // Public test vector from minisign-verify 0.2.4.
        let pubkey = STANDARD.encode(concat!(
            "untrusted comment: minisign public key\n",
            "RWQf6LRCGA9i53mlYecO4IzT51TGPpvWucNSCh1CBM0QTaLn73Y7GFO3",
        ));
        let signature = STANDARD.encode(concat!(
            "untrusted comment: signature from minisign secret key\n",
            "RUQf6LRCGA9i559r3g7V1qNyJDApGip8MfqcadIgT9CuhV3EMhHoN1mGTkUidF/",
            "z7SrlQgXdy8ofjb7bNJJylDOocrCo8KLzZwo=\n",
            "trusted comment: timestamp:1556193335\tfile:test\n",
            "y/rUw2y8/hOUYjZU71eHp/Wo1KZ40fGy2VJEDl34XMJM+TX48Ss/17u3IvIfbVR1",
            "FkZZSNCisQbuQY+bHwhEBg==",
        ));

        assert!(verify_update_signature(b"test", &signature, &pubkey).is_ok());
        assert!(
            verify_update_signature(b"tampered", &signature, &pubkey).is_err()
        );
        assert!(verify_update_signature(b"test", "invalid", &pubkey).is_err());
        assert!(verify_update_signature(b"test", &signature, "invalid").is_err());
    }
}
