use crate::install::{InstallJobSnapshot, model::InstanceVersionSwitch, runner};
use crate::state::instances::adapters::sqlite::content_rows;
use crate::state::{CachedEntry, InstanceInstallStage, InstanceLink, ModLoader, ProjectType, State, Version};
use crate::{ErrorKind, Result};
use anyhow::{Context, ensure};
use serde::Serialize;
use std::collections::{HashMap, HashSet};

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
enum CompatibilityIssue {
	GameVersion,
	ModLoader,
	Unverified,
}

#[derive(Serialize)]
pub struct VersionSwitchContent {
	pub path: String,
	pub name: String,
	issue: CompatibilityIssue,
	pub can_disable: bool,
}

pub async fn preview_instance_version_switch(
	instance_id: &str,
	game_version: &str,
	loader: ModLoader,
) -> Result<Vec<VersionSwitchContent>> {
	let state = State::get().await?;
	let metadata = crate::state::get_instance(instance_id, &state.pool)
		.await?
		.context("unknown instance")?;
	ensure!(
		!metadata.quarantined && metadata.instance.install_stage == InstanceInstallStage::Installed
			&& matches!(metadata.link, InstanceLink::Unmanaged),
		"only installed, unmanaged instances can change Minecraft or loader versions this way"
	);
	let files = crate::state::sync_content_files(instance_id, &state).await?;
	let entries = content_rows::get_content_entries(&metadata.applied_content_set.id, &state.pool).await?;
	let versions_by_file: HashMap<_, _> = entries.iter()
		.filter_map(|entry| Some((entry.file_id.as_deref()?, entry.version_id.as_deref()?)))
		.collect();
	let ids: Vec<&str> = versions_by_file.values().copied().collect::<HashSet<_>>().into_iter().collect();
	let versions = match CachedEntry::get_version_many(&ids, None, &state.pool, &state.api_semaphore).await {
		Ok(versions) => versions,
		Err(error) => {
			tracing::debug!("Could not verify content versions before switching {instance_id}: {error}");
			Vec::new()
		}
	};
	let versions: HashMap<_, _> = versions.iter().map(|version| (version.id.as_str(), version)).collect();
	let locks = content_rows::get_locked_instance_file_ids(instance_id, &state.pool).await?;
	let mut issues = Vec::new();
	for file in files.into_iter().filter(|file| file.enabled && !file.missing) {
		let Some(project_type) = ProjectType::get_from_parent_folder(&file.relative_path) else {
			continue;
		};
		let exact_version = versions_by_file.get(file.id.as_str()).and_then(|id| versions.get(id)).copied();
		if let Some(issue) = issue_for_version(exact_version, project_type, game_version, loader) {
			let can_disable = project_type == ProjectType::Mod
				&& !locks.contains(&file.id)
				&& !matches!(issue, CompatibilityIssue::Unverified);
			issues.push(VersionSwitchContent {
				path: file.relative_path,
				name: file.file_name,
				issue,
				can_disable,
			});
		}
	}
	issues.sort_by(|a, b| a.path.cmp(&b.path));
	Ok(issues)
}

fn issue_for_version(
	version: Option<&Version>,
	project_type: ProjectType,
	game_version: &str,
	loader: ModLoader,
) -> Option<CompatibilityIssue> {
	if project_type == ProjectType::Mod && loader == ModLoader::Vanilla {
		return Some(CompatibilityIssue::ModLoader);
	}
	let Some(version) = version else {
		return Some(CompatibilityIssue::Unverified);
	};
	if !version.game_versions.is_empty() && !version.game_versions.iter().any(|v| v == game_version) {
		return Some(CompatibilityIssue::GameVersion);
	}
	if project_type == ProjectType::Mod
		&& !version.loaders.is_empty()
		&& !version.loaders.iter().any(|v| v == loader.as_str())
	{
		return Some(CompatibilityIssue::ModLoader);
	}
	if version.game_versions.is_empty() || (project_type == ProjectType::Mod && version.loaders.is_empty()) {
		return Some(CompatibilityIssue::Unverified);
	}
	None
}

pub async fn switch_instance_version(
	instance_id: String,
	game_version: String,
	loader: ModLoader,
	loader_version: Option<String>,
	disable_mods: Vec<String>,
) -> Result<InstallJobSnapshot> {
	ensure!(!game_version.trim().is_empty(), "Minecraft version is required");
	if !disable_mods.is_empty() {
		let allowed: HashSet<_> = preview_instance_version_switch(&instance_id, &game_version, loader)
			.await?
			.into_iter()
			.filter(|item| item.can_disable)
			.map(|item| item.path)
			.collect();
		if !disable_mods.iter().all(|path| allowed.contains(path)) {
			return Err(ErrorKind::InputError(
				"installed content changed since the compatibility preview; check it again".into()
			).into());
		}
	}
	runner::switch_instance_version(instance_id, InstanceVersionSwitch {
		game_version,
		loader,
		loader_version: if loader == ModLoader::Vanilla { None } else { loader_version },
		disable_mods,
	}).await
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn unknown_mod_is_flagged_and_vanilla_cannot_load_it() {
		assert!(matches!(
			issue_for_version(None, ProjectType::Mod, "1.21.1", ModLoader::Fabric),
			Some(CompatibilityIssue::Unverified)
		));
		assert!(matches!(
			issue_for_version(None, ProjectType::Mod, "1.21.1", ModLoader::Vanilla),
			Some(CompatibilityIssue::ModLoader)
		));
	}
}
