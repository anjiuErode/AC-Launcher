use std::{future::Future, path::PathBuf};

use serde::{Deserialize, Serialize};

use crate::{
    install::{InstallPhaseDetails, InstallProgressReporter},
    state::ModLoader,
};

use super::{
    curseforge_archive::{self, ManualFiles},
    launcher_directory::{self, FolderLayout, LauncherDirectoryType},
};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ExternalImportSource {
    LauncherDirectory {
        path: PathBuf,
        launcher: LauncherDirectoryType,
        version_id: String,
        layout: FolderLayout,
    },
    CurseForgeArchive {
        path: PathBuf,
        manual: Option<ManualFiles>,
    },
    CurseForgeOnline {
        project_id: u32,
        file_id: u32,
        manual: Option<ManualFiles>,
    },
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstanceSpec {
    pub name: String,
    pub game_version: String,
    pub loader: ModLoader,
    pub loader_version: Option<String>,
}

pub(crate) fn prepare(
    source: ExternalImportSource,
    instances: PathBuf,
    staging: PathBuf,
    reporter: InstallProgressReporter,
    details: InstallPhaseDetails,
) -> impl Future<Output = crate::Result<InstanceSpec>> + Send {
    Box::pin(async move {
        match source {
            ExternalImportSource::LauncherDirectory {
                path,
                launcher,
                version_id,
                layout,
            } => {
                launcher_directory::prepare(
                    path, launcher, version_id, layout, instances,
                    staging.join("payload"),
                ).await
            }
            ExternalImportSource::CurseForgeArchive { path, manual } => {
                curseforge_archive::prepare(
                    Some(path), None, manual, staging, reporter, details,
                ).await
            }
            ExternalImportSource::CurseForgeOnline {
                project_id,
                file_id,
                manual,
            } => {
                curseforge_archive::prepare(
                    None, Some((project_id, file_id)), manual, staging,
                    reporter, details,
                ).await
            }
        }
    })
}

pub(super) fn normalize_loader_version(
    loader: ModLoader,
    game_version: &str,
    version: &str,
) -> String {
    let mut version = version.trim();
    if matches!(loader, ModLoader::Forge | ModLoader::NeoForge) {
        version = version.strip_prefix(game_version)
            .and_then(|suffix| suffix.strip_prefix('-'))
            .unwrap_or(version);
    }
    if loader == ModLoader::Forge {
        version = version.strip_suffix(game_version)
            .and_then(|prefix| prefix.strip_suffix('-'))
            .unwrap_or(version);
    }
    version.to_owned()
}
