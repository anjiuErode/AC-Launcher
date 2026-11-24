use serde::Serialize;

#[derive(Debug, Serialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum HotfixCheck {
    Unsupported,
    #[cfg(all(feature = "updater", windows, target_arch = "x86_64"))]
    Unavailable,
    #[cfg(all(feature = "updater", windows, target_arch = "x86_64"))]
    Available { hotfix: windows::Hotfix },
}

#[tauri::command]
pub async fn check_acl_hotfix(app: tauri::AppHandle) -> crate::api::Result<HotfixCheck> {
    #[cfg(all(feature = "updater", windows, target_arch = "x86_64"))]
    if crate::are_updates_enabled() {
        return windows::check(app).await;
    }

    #[cfg(not(all(feature = "updater", windows, target_arch = "x86_64")))]
    let _ = app;

    Ok(HotfixCheck::Unsupported)
}

#[cfg(all(feature = "updater", windows, target_arch = "x86_64"))]
mod windows {
    use super::HotfixCheck;
    use anyhow::{Context, Result, anyhow, ensure};
    use base64::Engine;
    use base64::engine::general_purpose::STANDARD;
    use minisign_verify::{PublicKey, Signature};
    use semver::Version;
    use serde::{Deserialize, Serialize};
    use std::time::Duration;
    use tauri_plugin_updater::{Config, Update, UpdaterExt};
    use theseus::ProxyMode;
    use url::Url;

    const OFFICIAL_REPOSITORY: &str = "https://github.com/anjiuErode/AC-Launcher";
    const HOTFIX_FEED: &str =
        "https://github.com/anjiuErode/AC-Launcher/releases/download/hotfix-feed/latest.json";

    #[derive(Debug, Serialize)]
    #[serde(rename_all = "camelCase")]
    pub struct Hotfix {
        version: String,
        base_version: String,
        notes: Option<String>,
        release_url: String,
    }

    #[derive(Deserialize)]
    struct HotfixManifest {
        channel: String,
        base_version: String,
        version: String,
    }

    impl HotfixManifest {
        fn check(&self, current: &Version, download_url: &Url) -> Result<bool> {
            ensure!(self.channel == "hotfix", "Invalid hotfix channel");
            let base = Version::parse(&self.base_version)
                .context("Invalid hotfix base version")?;
            let version = Version::parse(&self.version)
                .context("Invalid hotfix version")?;
            ensure!(
                base.pre.is_empty()
                    && base.build.is_empty()
                    && same_hotfix_series(&base, &version),
                "Invalid hotfix version series"
            );
            ensure!(
                download_url.as_str() == format!(
                    "{OFFICIAL_REPOSITORY}/releases/download/hotfix-v{version}/ACL_{version}_x64-setup.exe"
                ),
                "Unexpected hotfix installer URL"
            );
            Ok(current == &base
                || (same_hotfix_series(&base, current) && current < &version))
        }
    }

    fn same_hotfix_series(base: &Version, version: &Version) -> bool {
        base.major == version.major
            && base.minor == version.minor
            && base.patch.checked_add(1) == Some(version.patch)
            && version.build.is_empty()
            && version
                .pre
                .as_str()
                .strip_prefix("hotfix.")
                .is_some_and(|sequence| {
                    !sequence.is_empty()
                        && sequence != "0"
                        && sequence.bytes().all(|digit| digit.is_ascii_digit())
                })
    }

    fn signature_matches_update_key(signature: &str, public_key: &str) -> Result<()> {
        let public_key = PublicKey::decode(&String::from_utf8(STANDARD.decode(public_key)?)?)?;
        let signature = Signature::decode(&String::from_utf8(STANDARD.decode(signature)?)?)?;
        // Discovery can validate the signature format and key ID without fetching the installer.
        let _ = public_key.verify_stream(&signature)?;
        Ok(())
    }

    fn inspect_update(current: &Version, update: Update, key: &str) -> Result<HotfixCheck> {
        let manifest: HotfixManifest = serde_json::from_value(update.raw_json)
            .context("Invalid hotfix manifest")?;
        let applicable = manifest.check(current, &update.download_url)?;
        if !applicable {
            return Ok(HotfixCheck::Unavailable);
        }
        signature_matches_update_key(&update.signature, key)
            .context("Invalid hotfix installer signature metadata")?;

        Ok(HotfixCheck::Available {
            hotfix: Hotfix {
                release_url: format!(
                    "{OFFICIAL_REPOSITORY}/releases/tag/hotfix-v{}",
                    manifest.version
                ),
                version: manifest.version,
                base_version: manifest.base_version,
                notes: update.body,
            },
        })
    }

    pub async fn check(app: tauri::AppHandle) -> crate::api::Result<HotfixCheck> {
        let config: Config = serde_json::from_value(
            app.config()
                .plugins
                .0
                .get("updater")
                .cloned()
                .context("Missing updater configuration")?,
        )
        .context("Invalid updater configuration")?;
        let proxy = theseus::http_proxy_config();
        let mut updater = app
            .updater_builder()
            .endpoints(vec![HOTFIX_FEED.parse()?])?
            .target("windows-x86_64")
            .timeout(Duration::from_secs(15))
            .configure_client(|client| {
                client
                    .https_only(true)
                    .danger_accept_invalid_certs(false)
                    .danger_accept_invalid_hostnames(false)
            })
            // Inspect the channel metadata even when the remote version is older.
            .version_comparator(|_, _| true);
        match proxy.mode {
            ProxyMode::Auto => {}
            ProxyMode::Off => updater = updater.no_proxy(),
            ProxyMode::Manual => {
                let url = proxy
                    .url
                    .as_deref()
                    .and_then(|url| url.parse().ok())
                    .ok_or_else(|| anyhow!("Invalid hotfix check proxy"))?;
                updater = updater.proxy(url);
            }
        }
        let update = updater
            .build()?
            .check()
            .await
            .map_err(|_| anyhow!("Could not load the hotfix release manifest"))?;
        let Some(update) = update else {
            return Ok(HotfixCheck::Unavailable);
        };
        Ok(inspect_update(&app.package_info().version, update, &config.pubkey)?)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn manifest(version: &str) -> HotfixManifest {
            HotfixManifest {
                channel: "hotfix".into(),
                base_version: "1.0.0".into(),
                version: version.into(),
            }
        }

        fn asset_url(version: &str) -> Url {
            format!("{OFFICIAL_REPOSITORY}/releases/download/hotfix-v{version}/ACL_{version}_x64-setup.exe")
                .parse()
                .unwrap()
        }

        #[test]
        fn only_newer_hotfixes_for_the_same_base_apply() {
            let version = "1.0.1-hotfix.10";
            let manifest = manifest(version);
            let url = asset_url(version);
            for (current, applicable) in [
                ("1.0.0", true),
                ("1.0.1-hotfix.9", true),
                ("0.9.9", false),
                ("1.0.0-local", false),
                ("1.0.1-beta.1", false),
                ("1.0.1-hotfix.10", false),
                ("1.0.1-hotfix.11", false),
                ("1.0.1", false),
                ("2.0.0", false),
            ] {
                assert_eq!(
                    manifest.check(&Version::parse(current).unwrap(), &url).unwrap(),
                    applicable,
                    "{current}"
                );
            }
        }

        #[test]
        fn rejects_versions_outside_the_hotfix_schema() {
            for version in [
                "1.0.1",
                "1.0.1-hotfix.0",
                "1.0.1-hotfix.+build",
                "1.0.1-hotfix.1+build",
                "1.0.1-hotfix.01",
                "1.0.1-hotfix.1.extra",
                "1.0.2-hotfix.1",
                "1.1.1-hotfix.1",
                "2.0.1-hotfix.1",
                "1.0.1-rc.1",
            ] {
                assert!(
                    manifest(version).check(&Version::new(1, 0, 0), &asset_url(version)).is_err(),
                    "{version}"
                );
            }
        }

        #[test]
        fn rejects_other_channels_and_noncanonical_installers() {
            let version = "1.0.1-hotfix.1";
            let base = Version::new(1, 0, 0);
            let mut manifest = manifest(version);
            let exact_url = asset_url(version);
            for bad_url in [
                exact_url.as_str().replace("https:", "http:"),
                exact_url.as_str().replace("github.com", "example.com"),
                exact_url.as_str().replace("/hotfix-v", "/v"),
                exact_url.as_str().replace("x64-setup.exe", "x64.msi"),
                format!("{exact_url}?download=1"),
            ] {
                assert!(manifest.check(&base, &bad_url.parse().unwrap()).is_err());
            }
            manifest.channel = "stable".into();
            assert!(manifest.check(&base, &exact_url).is_err());
            manifest.channel = "hotfix".into();
            manifest.base_version = "1.0.0-beta.1".into();
            assert!(manifest.check(&base, &exact_url).is_err());
        }

        #[test]
        fn rejects_missing_channel_and_base_version() {
            for raw_manifest in [
                serde_json::json!({"version": "1.0.1-hotfix.1", "base_version": "1.0.0"}),
                serde_json::json!({"version": "1.0.1-hotfix.1", "channel": "hotfix"}),
                serde_json::json!({"version": "1.0.1-hotfix.1", "channel": "hotfix", "base_version": 1}),
            ] {
                assert!(serde_json::from_value::<HotfixManifest>(raw_manifest).is_err());
            }
        }

        #[test]
        fn checks_signature_format_and_configured_key_id() {
            // Public test vector from minisign-verify 0.2.4.
            let public_key = STANDARD.encode(concat!(
                "untrusted comment: minisign public key\n",
                "RWQf6LRCGA9i53mlYecO4IzT51TGPpvWucNSCh1CBM0QTaLn73Y7GFO3", // typos:ignore
            ));
            let signature = STANDARD.encode(concat!(
                "untrusted comment: signature from minisign secret key\n",
                "RUQf6LRCGA9i559r3g7V1qNyJDApGip8MfqcadIgT9CuhV3EMhHoN1mGTkUidF/",
                "z7SrlQgXdy8ofjb7bNJJylDOocrCo8KLzZwo=\n",
                "trusted comment: timestamp:1556193335\tfile:test\n",
                "y/rUw2y8/hOUYjZU71eHp/Wo1KZ40fGy2VJEDl34XMJM+TX48Ss/17u3IvIfbVR1",
                "FkZZSNCisQbuQY+bHwhEBg==",
            ));
            let wrong_key = STANDARD.encode(concat!(
                "untrusted comment: minisign public key with different ID\n",
                "RWQe6LRCGA9i53mlYecO4IzT51TGPpvWucNSCh1CBM0QTaLn73Y7GFO3", // typos:ignore
            ));
            assert!(signature_matches_update_key(&signature, &public_key).is_ok());
            assert!(signature_matches_update_key(&signature, &wrong_key).is_err());
            assert!(signature_matches_update_key("invalid", &public_key).is_err());
        }
    }
}
