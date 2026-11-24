use super::archive_io;
use crate::event::{InstancePayloadType, emit::emit_instance};
use crate::state::content_store::{
	FileContent, FileStorageKind, InstanceFileStatus, content_file_path,
	hash_file, is_managed_content_path, validate_digest,
};
use crate::state::instances::adapters::sqlite::{content_rows, instance_rows};
use crate::state::{
	ContentEntry, ContentSet, ContentSetStatus, ContentSourceKind, Instance,
	InstanceFile, InstanceIconConfig, InstanceInstallStage, InstanceLaunchOverrides,
	InstanceLink, InstanceSyncedOption, InstanceTabVisibility, LauncherFeatureVersion,
	MemorySettings, ReleaseChannel, State, WindowSize,
};
use crate::{ErrorKind, Result};
use anyhow::{Context, ensure};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use uuid::Uuid;

const MANIFEST: &str = "acl-instance.json";
const INSTANCE_FILES: &str = "instance";
const INSTANCE_ICON: &str = "instance-icon.png";
const MAX_ICON_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Serialize, Deserialize)]
struct InstanceArchive {
	format_version: u32,
	name: String,
	update_channel: ReleaseChannel,
	icon_config: Option<InstanceIconConfig>,
	active_set: String,
	sets: Vec<ContentSet>,
	files: Vec<ArchiveContentFile>,
	entries: Vec<ContentEntry>,
	memory: Option<MemorySettings>,
	force_fullscreen: Option<bool>,
	game_resolution: Option<WindowSize>,
	visible_tabs: InstanceTabVisibility,
}

#[derive(Serialize, Deserialize)]
struct ArchiveContentFile {
	file: InstanceFile,
	locked: bool,
	digest: Option<String>,
	managed: bool,
}

pub async fn export_instance_archive(instance_id: &str, output: &Path) -> Result<()> {
	let state = State::get().await?;
	let _activity = state.instance_locks.lock_activity(instance_id).await;
	let _content = state.instance_locks.lock_content(instance_id).await;
	let metadata = crate::state::get_instance(instance_id, &state.pool)
		.await?
		.context("unknown instance")?;
	ensure!(
		!metadata.quarantined && metadata.instance.install_stage == InstanceInstallStage::Installed,
		"the instance is unavailable for archiving"
	);
	ensure!(
		!crate::state::instance_has_running_process(instance_id, &state).await?,
		"stop Minecraft before exporting an instance"
	);
	let source = state.directories.instances_dir().join(&metadata.instance.path);
	archive_io::checked_dir(&source)?;
	let canonical_source = std::fs::canonicalize(&source)?;
	let output_parent = std::fs::canonicalize(output.parent().context("archive output has no parent folder")?)?;
	ensure!(
		!output_parent.starts_with(&canonical_source),
		"save the archive outside the instance folder"
	);
	let output = output_parent.join(output.file_name().context("archive output has no filename")?);
	let files = content_rows::get_instance_files(instance_id, &state.pool).await?;
	let locked = content_rows::get_locked_instance_file_ids(instance_id, &state.pool).await?;
	let bindings: HashMap<_, _> = crate::state::content_store::instance_storage(&state.pool, instance_id)
		.await?
		.into_iter()
		.map(|binding| (binding.file_id.clone(), binding))
		.collect();
	let mut source_files = HashMap::new();
	let mut archive_files = Vec::with_capacity(files.len());
	for mut file in files {
		let path = source.join(archive_io::archive_path(&content_file_path(&file))?);
		let binding = bindings.get(&file.id);
		let mut link_source = None;
		let file_metadata = match tokio::fs::symlink_metadata(&path).await {
			Ok(metadata) => Some(metadata),
			Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
			Err(error) => return Err(error.into()),
		};
		file.missing = file_metadata.is_none();
		let digest = if let Some(file_metadata) = file_metadata {
			archive_io::checked_parent_dirs(&source, &path)?;
			if let Some(binding) = binding {
				ensure!(
					state.content_store.check_instance_file(&metadata.instance, &file, binding).await?
						== InstanceFileStatus::Healthy,
					"repair {} before exporting the instance", file.relative_path
				);
				if archive_io::path_is_link(&file_metadata) {
					let FileContent::Stored { stored_file, .. } = state.content_store.file_content(&file).await? else {
						return Err(ErrorKind::InputError(format!("missing stored file: {}", file.relative_path)).into());
					};
					link_source = Some(stored_file.path.clone());
					source_files.insert(path.clone(), stored_file.path.clone());
				}
			}
			let actual = link_source.as_deref().unwrap_or(&path);
			ensure!(link_source.is_some() || !archive_io::path_is_link(&file_metadata), "instance contains an untracked link: {}", file.relative_path);
			ensure!(tokio::fs::metadata(actual).await?.is_file(), "invalid content file: {}", file.relative_path);
			let hashes = hash_file(actual).await?;
			file.size = hashes.size;
			file.sha1 = crate::util::fetch::sha1_file_async(actual).await?.1;
			Some(hashes.sha512)
		} else {
			None
		};
		archive_files.push(ArchiveContentFile {
			locked: locked.contains(&file.id),
			file,
			managed: binding.is_some(),
			digest,
		});
	}
	let sets = content_rows::get_content_sets_for_instance(instance_id, &state.pool).await?;
	let mut entries = Vec::new();
	for set in &sets {
		entries.extend(content_rows::get_content_entries(&set.id, &state.pool).await?);
	}
	let manifest = InstanceArchive {
		format_version: 1,
		name: metadata.instance.name,
		update_channel: metadata.instance.update_channel,
		icon_config: metadata.icon_config,
		active_set: metadata.applied_content_set.id,
		sets,
		files: archive_files,
		entries,
		memory: metadata.launch_overrides.memory,
		force_fullscreen: metadata.launch_overrides.force_fullscreen,
		game_resolution: metadata.launch_overrides.game_resolution,
		visible_tabs: metadata.launch_overrides.visible_tabs,
	};
	let icon = if let Some(path) = metadata.instance.icon_path {
		let path = PathBuf::from(path);
		match tokio::fs::symlink_metadata(&path).await {
			Ok(info) if info.is_file() && !archive_io::path_is_link(&info) && info.len() <= MAX_ICON_BYTES => {
				Some(tokio::fs::read(path).await?)
			}
			Ok(_) => None,
			Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
			Err(error) => return Err(error.into()),
		}
	} else {
		None
	};
	let bytes = serde_json::to_vec(&manifest)?;
	ensure!(bytes.len() as u64 <= archive_io::MAX_MANIFEST_BYTES, "instance archive metadata is too large");
	let storage_lease = state.content_store.lease().await;
	tokio::task::spawn_blocking(move || {
		let _storage_lease = storage_lease;
		archive_io::write_archive(&output, |zip| {
			archive_io::write_bytes(zip, MANIFEST, &bytes)?;
			if let Some(icon) = icon {
				archive_io::write_bytes(zip, INSTANCE_ICON, &icon)?;
			}
			archive_io::write_tree_with_files(zip, &source, INSTANCE_FILES, &source_files)
		})
	})
	.await??;
	Ok(())
}

pub async fn restore_instance_archive(archive: &Path) -> Result<String> {
	let state = State::get().await?;
	let instances = state.directories.instances_dir();
	archive_io::checked_dir(&instances)?;
	let parent = instances.parent().context("instance folder has no parent")?.to_owned();
	let archive = archive.to_owned();
	let (stage, mut manifest) = tokio::task::spawn_blocking(move || {
		let stage = tempfile::Builder::new().prefix(".acl-restore-").tempdir_in(parent)?;
		archive_io::extract_archive_with_limits(
			&archive,
			stage.path(),
			&[(MANIFEST, archive_io::MAX_MANIFEST_BYTES), (INSTANCE_ICON, MAX_ICON_BYTES)],
		)?;
		let manifest = read_manifest(stage.path())?;
		Ok::<_, crate::Error>((stage, manifest))
	}).await??;
	let staged_files = stage.path().join(INSTANCE_FILES);
	let archived_icon = stage.path().join(INSTANCE_ICON);
	if let Some(icon_config) = &manifest.icon_config {
		super::icon::validate_generated_icon_config(icon_config)?;
	}
	let icon_path = if archived_icon.is_file() {
		Some(super::icon::cache_icon_from_path(&archived_icon, &state)
			.await?.to_string_lossy().into_owned())
	} else {
		None
	};
	let id = format!("local:{}", Uuid::new_v4());
	let folder = format!("restored-{}", Uuid::new_v4());
	let mut set_ids = HashMap::new();
	for set in &mut manifest.sets {
		let new = format!("content-set:{}", Uuid::new_v4());
		ensure!(set_ids.insert(std::mem::replace(&mut set.id, new.clone()), new).is_none(), "duplicate content set in archive");
		set.instance_id.clone_from(&id);
		set.source_kind = ContentSourceKind::Local;
		set.status = ContentSetStatus::Available;
		ensure!(!set.game_version.is_empty(), "archive is missing its Minecraft version");
	}
	let active_set = set_ids.get(&manifest.active_set).cloned().context("archive has no active content set")?;
	let now = Utc::now();
	let instance = Instance {
		id: id.clone(),
		path: folder,
		applied_content_set_id: Some(active_set),
		install_stage: InstanceInstallStage::Installed,
		launcher_feature_version: LauncherFeatureVersion::MOST_RECENT,
		update_channel: manifest.update_channel,
		name: manifest.name,
		icon_path,
		created: now,
		modified: now,
		last_played: None,
		submitted_time_played: 0,
		recent_time_played: 0,
	};
	let mut launch_overrides = InstanceLaunchOverrides::empty(id.clone());
	launch_overrides.memory = manifest.memory;
	launch_overrides.force_fullscreen = manifest.force_fullscreen;
	launch_overrides.game_resolution = manifest.game_resolution;
	launch_overrides.visible_tabs = manifest.visible_tabs;

	let mut originals = HashMap::new();
	let mut file_paths = HashSet::new();
	let mut stored = Vec::new();
	for archived in &mut manifest.files {
		let file = &mut archived.file;
		let relative = content_file_path(file);
		ensure!(file_paths.insert(file.relative_path.to_uppercase()), "duplicate archived content path: {relative}");
		ensure!(file.size <= i64::MAX as u64, "archived content file is too large");
		ensure!(archive_io::archive_path(&file.file_name)?.components().count() == 1, "invalid archived content name");
		let path = staged_files.join(archive_io::archive_path(&relative)?);
		let new_id = format!("instance-file:{}", Uuid::new_v4());
		ensure!(
			originals.insert(std::mem::replace(&mut file.id, new_id.clone()), new_id).is_none(),
			"duplicate content reference in archive"
		);
		file.instance_id.clone_from(&id);
		if let Some(digest) = &archived.digest {
			ensure!(!file.missing, "archive flags an included file as missing: {relative}");
			validate_digest(digest, 128)?;
			let hashes = hash_file(&path).await?;
			ensure!(hashes.sha512 == *digest && hashes.size == file.size, "archived content changed: {relative}");
			file.sha1 = crate::util::fetch::sha1_file_async(&path).await?.1;
			if archived.managed {
				ensure!(is_managed_content_path(&relative), "invalid managed content path: {relative}");
				let blob = state.content_store.store_file(&path).await?;
				ensure!(blob.metadata.sha512 == *digest, "archived content changed: {relative}");
				stored.push((file.id.clone(), blob.metadata.sha512.clone()));
			}
		} else {
			ensure!(file.missing && !tokio::fs::try_exists(&path).await?, "archive is missing a content checksum: {relative}");
		}
	}
	for entry in &mut manifest.entries {
		entry.id = format!("content-entry:{}", Uuid::new_v4());
		entry.instance_id.clone_from(&id);
		entry.content_set_id = set_ids.get(&entry.content_set_id).cloned().context("unknown archived content set")?;
		if let Some(old_file) = &entry.file_id {
			entry.file_id = Some(originals.get(old_file).cloned().context("unknown archived content file")?);
		}
		entry.source_kind = ContentSourceKind::Local;
	}

	let _storage_lease = state.content_store.lease().await;
	let mut tx = state.pool.begin_with("BEGIN IMMEDIATE").await?;
	instance_rows::insert_instance(&instance, &mut tx).await?;
	for set in &manifest.sets {
		content_rows::insert_content_set(set, &mut tx).await?;
	}
	instance_rows::upsert_instance_link(&id, &InstanceLink::Unmanaged, &mut tx).await?;
	instance_rows::upsert_instance_launch_overrides(&launch_overrides, &mut tx).await?;
	if let Some(config) = manifest.icon_config.as_ref() {
		instance_rows::update_instance_icon_config(&id, Some(config), &mut tx).await?;
	}
	for preference in InstanceSyncedOption::ALL {
		sqlx::query("INSERT INTO instance_sync_preferences (instance_id, feature, enabled) VALUES (?, ?, 0)")
			.bind(&id).bind(preference.as_str()).execute(&mut *tx).await?;
	}
	for archived in &manifest.files {
		content_rows::upsert_instance_file(&archived.file, &mut tx).await?;
		if archived.locked {
			content_rows::insert_instance_file_lock(&archived.file.id, &mut tx).await?;
		}
	}
	for entry in &manifest.entries {
		content_rows::insert_content_entry(entry, &mut tx).await?;
	}
	for (file_id, digest) in &stored {
		crate::state::content_store::set_file_storage(&mut tx, file_id, digest, FileStorageKind::Copy).await?;
	}
	let destination = instances.join(&instance.path);
	ensure!(!tokio::fs::try_exists(&destination).await?, "restore destination already exists");
	tokio::fs::rename(&staged_files, &destination).await?;
	if let Err(error) = tx.commit().await {
		tokio::fs::rename(&destination, &staged_files).await.with_context(|| {
			format!("could not move the unregistered archive back to staging after a database error: {error}")
		})?;
		return Err(error.into());
	}
	crate::state::instances::watcher::watch_instance_folder(&id, &instance.path, &state.file_watcher, &state.directories).await;
	if let Err(error) = emit_instance(&id, InstancePayloadType::Created).await {
		tracing::warn!("Could not announce restored instance {id}: {error}");
	}
	if let Err(error) = crate::onboarding_checklist::mark_created_instance().await {
		tracing::warn!("Could not mark restored instance in onboarding checklist: {error}");
	}
	Ok(id)
}

fn read_manifest(root: &Path) -> Result<InstanceArchive> {
	for item in std::fs::read_dir(root)? {
		let item = item?;
		ensure!(
			matches!(item.file_name().to_str(), Some(MANIFEST | INSTANCE_FILES | INSTANCE_ICON)),
			"unexpected top-level file in instance archive"
		);
	}
	let manifest = root.join(MANIFEST);
	let info = std::fs::metadata(&manifest)?;
	ensure!(info.is_file() && info.len() <= archive_io::MAX_MANIFEST_BYTES, "invalid instance archive metadata");
	archive_io::checked_dir(&root.join(INSTANCE_FILES))?;
	if let Ok(info) = std::fs::metadata(root.join(INSTANCE_ICON)) {
		ensure!(info.is_file() && info.len() <= MAX_ICON_BYTES, "invalid instance archive icon");
	}
	let manifest: InstanceArchive = serde_json::from_slice(&std::fs::read(manifest)?)?;
	ensure!(manifest.format_version == 1, "unsupported ACL instance archive version");
	ensure!(!manifest.name.trim().is_empty(), "instance archive has no name");
	Ok(manifest)
}
