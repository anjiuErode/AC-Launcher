use std::{collections::{BTreeSet, HashMap, HashSet}, path::Path};

use anyhow::{Context, bail, ensure};
use serde::Serialize;

use crate::{
    event::{InstancePayloadType, emit::emit_instance},
    state::{
        ContentSourceKind, InstanceInstallStage, InstanceLink, InstanceMetadata,
        ModLoader, ProjectType, State,
        content_store::{FileContent, InstanceFileStatus},
        instances::{
            adapters::sqlite::content_rows,
            commands::{InstallContent, install_stored_file_if_absent_after},
        },
    },
};

use super::{
    curseforge_archive::{
        ApiFile, CurseForgeCategory, api_download_url, dependency_files,
        download_to, get_project_file, valid_sha1,
    },
    launcher_directory::is_single_name,
};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentInstallPreview {
    targets: Vec<ContentTarget>,
    required_projects: Vec<u32>,
    conflicting_projects: Vec<u32>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ContentTarget {
    id: String,
    name: String,
    game_version: String,
    loader: ModLoader,
    satisfied_projects: Vec<u32>,
}

pub async fn preview_content_install(
    category: CurseForgeCategory,
    project_id: u32,
    file_id: u32,
) -> crate::Result<ContentInstallPreview> {
    let project_type = content_type(category)?;
    let (project, file) = get_project_file(category, project_id, file_id).await?;
    validate_content_file(&file, project_type)?;
    ensure!(api_download_url(&project, &file).is_some(),
        "This file is unavailable for automatic CurseForge downloads.");
    let (required, conflicting) = relationship_blockers(&file);
    let state = State::get().await?;
    let mut targets = Vec::new();
    if conflicting.is_empty() {
        let dependencies = load_dependencies(&required).await;
        for instance in crate::state::list_instances(&state.pool).await? {
            if target_compatible(&instance, project_type, &file)
                && !crate::state::instance_has_running_process(&instance.instance.id, &state).await?
            {
                let satisfied = satisfied_dependencies(&instance, &dependencies, &state).await?;
                targets.push(ContentTarget {
                    id: instance.instance.id,
                    name: instance.instance.name,
                    game_version: instance.applied_content_set.game_version,
                    loader: instance.applied_content_set.loader,
                    satisfied_projects: satisfied.into_iter().collect(),
                });
            }
        }
        targets.sort_by(|left, right| left.name.cmp(&right.name).then_with(|| left.id.cmp(&right.id)));
    }
    Ok(ContentInstallPreview {
        targets,
        required_projects: required.into_iter().collect(),
        conflicting_projects: conflicting.into_iter().collect(),
    })
}

pub async fn install_content(
    category: CurseForgeCategory,
    project_id: u32,
    file_id: u32,
    instance_id: String,
    world: Option<String>,
) -> crate::Result<String> {
    let project_type = content_type(category)?;
    ensure!((project_type == ProjectType::DataPack) == world.is_some(),
        "Select a local world for the data pack.");
    let (project, file) = get_project_file(category, project_id, file_id).await?;
    validate_content_file(&file, project_type)?;
    let (required, conflicting) = relationship_blockers(&file);
    ensure!(conflicting.is_empty(), "CurseForge file conflicts could not be verified: {conflicting:?}.");
    let url = api_download_url(&project, &file)
        .context("This file is unavailable for automatic CurseForge downloads.")?;
    let sha1 = valid_sha1(&file).context("The CurseForge file is missing an exact SHA-1.")?;
    let state = State::get().await?;
    let metadata = require_compatible_target(&instance_id, project_type, &file, &state).await?;
    let dependencies = load_dependencies(&required).await;
    verify_dependencies(&metadata, &required, &dependencies, &state).await?;

    let staging = tempfile::tempdir()?;
    let downloaded = staging.path().join(&file.file_name);
    download_to(&url, &downloaded, file.file_length, sha1).await?;

    let _activity = state.instance_locks.lock_activity(&instance_id).await;
    let _install_target = crate::install::runner::reserve_external_content_target(&instance_id)?;
    let metadata = require_compatible_target(&instance_id, project_type, &file, &state).await?;
    let revalidate = verify_dependencies(&metadata, &required, &dependencies, &state);
    let relative = if let Some(world) = world {
        let _content = state.instance_locks.lock_content(&instance_id).await;
        revalidate.await?;
        let directory = state.directories.instances_dir().join(&metadata.instance.path);
        crate::api::worlds::install_datapack_in_world(&directory, &world, &downloaded).await?
    } else {
        let relative = format!("{}/{}", project_type.get_folder(), file.file_name);
        let stored = state.content_store.store_file(&downloaded).await?;
        install_stored_file_if_absent_after(&instance_id, InstallContent {
            requested_path: &relative,
            stored_file: &stored,
            project_type,
            source_kind: ContentSourceKind::Local,
            origin: None,
            enabled_override: None,
            previous_path: None,
        }, &state, revalidate).await?;
        if project_type == ProjectType::ResourcePack {
            crate::api::instance::synced_packs::reconcile_after_content_change(&instance_id).await;
        }
        relative
    };
    if let Err(error) = emit_instance(&instance_id, InstancePayloadType::Edited).await {
        tracing::warn!(%error, "Unable to refresh instance after installing a CurseForge file");
    }
    Ok(relative)
}

async fn require_compatible_target(
    id: &str,
    project_type: ProjectType,
    file: &ApiFile,
    state: &State,
) -> crate::Result<InstanceMetadata> {
    let metadata = crate::state::get_instance(id, &state.pool).await?
        .context("The selected ACL instance is unavailable.")?;
    ensure!(target_compatible(&metadata, project_type, file),
        "Select an installed, unmanaged ACL instance with a compatible Minecraft version and mod loader.");
    ensure!(!crate::state::instance_has_running_process(id, state).await?,
        "Stop Minecraft before installing CurseForge content.");
    Ok(metadata)
}

fn target_compatible(metadata: &InstanceMetadata, project_type: ProjectType, file: &ApiFile) -> bool {
    !metadata.quarantined
        && metadata.instance.install_stage == InstanceInstallStage::Installed
        && matches!(metadata.link, InstanceLink::Unmanaged)
        && metadata.shared_instance.is_none()
        && metadata.applied_content_set.source_kind == ContentSourceKind::Local
        && runtime_compatible(project_type, &file.game_versions, &metadata.applied_content_set.game_version, metadata.applied_content_set.loader)
}

fn runtime_compatible(project_type: ProjectType, tags: &[String], game_version: &str, loader: ModLoader) -> bool {
    if !tags.iter().any(|tag| tag == game_version) {
        return false;
    }
    let explicit_loader = tags.iter().any(|tag| {
        matches!(tag.to_ascii_lowercase().as_str(), "forge" | "fabric" | "quilt" | "neoforge" | "liteloader" | "cauldron" | "rift")
    });
    if project_type == ProjectType::Mod && loader == ModLoader::Vanilla {
        return false;
    }
    if explicit_loader || project_type == ProjectType::Mod {
        return tags.iter().any(|tag| tag.eq_ignore_ascii_case(loader.as_str()));
    }
    true
}

fn relationship_blockers(file: &ApiFile) -> (BTreeSet<u32>, BTreeSet<u32>) {
    let mut required = BTreeSet::new();
    let mut conflicting = BTreeSet::new();
    for relation in file.dependencies() {
        match relation.relation_type {
            3 => { required.insert(relation.mod_id); }
            1 | 2 | 4 | 6 => {}
            _ => { conflicting.insert(relation.mod_id); }
        }
    }
    (required, conflicting)
}

async fn load_dependencies(projects: &BTreeSet<u32>) -> HashMap<u32, Vec<ApiFile>> {
    const MAX_PROJECTS: usize = 8;
    let mut tasks = tokio::task::JoinSet::new();
    for &project in projects.iter().take(MAX_PROJECTS) {
        tasks.spawn(async move { (project, dependency_files(project).await) });
    }
    let mut files = HashMap::new();
    while let Some(result) = tasks.join_next().await {
        if let Ok((project, Ok(found))) = result {
            files.insert(project, found);
        }
    }
    files
}

async fn verify_dependencies(
    metadata: &InstanceMetadata,
    required: &BTreeSet<u32>,
    dependencies: &HashMap<u32, Vec<ApiFile>>,
    state: &State,
) -> crate::Result<()> {
    let satisfied = satisfied_dependencies(metadata, dependencies, state).await?;
    let missing: Vec<_> = required.difference(&satisfied).copied().collect();
    ensure!(missing.is_empty(), "Required CurseForge projects could not be verified in this instance: {missing:?}.");
    Ok(())
}

async fn satisfied_dependencies(
    metadata: &InstanceMetadata,
    dependencies: &HashMap<u32, Vec<ApiFile>>,
    state: &State,
) -> crate::Result<BTreeSet<u32>> {
    let mut satisfied = BTreeSet::new();
    if dependencies.is_empty() { return Ok(satisfied); }
    let runtime = &metadata.applied_content_set;
    let mut expected = HashMap::<(String, u64), Vec<u32>>::new();
    for (project, files) in dependencies {
        for file in files {
            if let Some(hash) = valid_sha1(file)
                && runtime_compatible(ProjectType::Mod, &file.game_versions, &runtime.game_version, runtime.loader)
            {
                expected.entry((hash.to_ascii_lowercase(), file.file_length)).or_default().push(*project);
            }
        }
    }
    if expected.is_empty() { return Ok(satisfied); }
    let mut checked = HashSet::new();
    for file in content_rows::get_instance_files(&metadata.instance.id, &state.pool).await? {
        if !file.enabled || file.missing || ProjectType::get_from_parent_folder(&file.relative_path) != Some(ProjectType::Mod) {
            continue;
        }
        let key = (file.sha1.to_ascii_lowercase(), file.size);
        let Some(projects) = expected.get(&key) else { continue; };
        if checked.contains(&key) { continue; }
        if let FileContent::Stored { storage, stored_file } = state.content_store.file_content(&file).await?
            && state.content_store.check_instance_file(&metadata.instance, &file, &storage).await? == InstanceFileStatus::Healthy
        {
            let (size, sha1) = crate::util::fetch::sha1_file_async(&stored_file.path).await?;
            if (sha1, size) == key {
                satisfied.extend(projects);
                checked.insert(key);
            }
        }
    }
    Ok(satisfied)
}

fn content_type(category: CurseForgeCategory) -> crate::Result<ProjectType> {
    Ok(match category {
        CurseForgeCategory::Mod => ProjectType::Mod,
        CurseForgeCategory::ResourcePack => ProjectType::ResourcePack,
        CurseForgeCategory::Shader => ProjectType::ShaderPack,
        CurseForgeCategory::DataPack => ProjectType::DataPack,
        CurseForgeCategory::Modpack => bail!("Install CurseForge modpacks as new instances."),
    })
}

fn validate_content_file(file: &ApiFile, project_type: ProjectType) -> crate::Result<()> {
    ensure!(is_single_name(&file.file_name), "Invalid CurseForge filename.");
    crate::api::instance::archive_io::archive_path(&file.file_name)?;
    let extension = Path::new(&file.file_name).extension().and_then(|extension| extension.to_str());
    let expected = if project_type == ProjectType::Mod { "jar" } else { "zip" };
    ensure!(extension.is_some_and(|extension| extension.eq_ignore_ascii_case(expected)),
        "The selected file is not a supported {expected} for this category.");
    ensure!(valid_sha1(file).is_some() && file.file_length > 0,
        "The CurseForge file is missing an exact length or SHA-1.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn curseforge_catalog_uses_exact_game_version_and_loader() {
        let tags = ["1.20.1".to_owned(), "Fabric".to_owned()];
        assert!(runtime_compatible(ProjectType::Mod, &tags, "1.20.1", ModLoader::Fabric));
        assert!(!runtime_compatible(ProjectType::Mod, &tags, "1.20.1", ModLoader::Forge));
        assert!(!runtime_compatible(ProjectType::Mod, &tags, "1.20", ModLoader::Fabric));
        assert!(!runtime_compatible(ProjectType::Mod, &tags[..1], "1.20.1", ModLoader::Fabric));
        assert!(runtime_compatible(ProjectType::ResourcePack, &tags[..1], "1.20.1", ModLoader::Vanilla));
    }

    #[test]
    fn curseforge_relationships_distinguish_required_optional_and_conflicting() -> crate::Result<()> {
        let fixture: ApiFile = serde_json::from_value(serde_json::json!({
            "id": 88, "gameId": 432, "modId": 55,
            "displayName": "fixture", "fileName": "fixture.jar", "fileLength": 3,
            "isAvailable": true, "gameVersions": ["1.20.1", "Fabric"],
            "dependencies": [
                { "modId": 7, "relationType": 2 },
                { "modId": 8, "relationType": 3 },
                { "modId": 9, "relationType": 5 }
            ]
        }))?;
        let (required, conflicting) = relationship_blockers(&fixture);
        assert_eq!(required, BTreeSet::from([8]));
        assert_eq!(conflicting, BTreeSet::from([9]));
        Ok(())
    }
}
