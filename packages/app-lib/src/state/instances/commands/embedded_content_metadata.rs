use crate::ErrorKind;
use crate::state::{
    CacheValue, CachedEmbeddedContentMetadata, CachedEntry, ContentFile,
    EmbeddedContentMetadata, Instance, ModLoader, ProjectType, State,
};
use anyhow::Context;
use bytes::Bytes;
use futures::stream::{self, StreamExt};
use serde_json::Value as JsonValue;
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{Cursor, Read, Seek};
use std::path::Path;
use toml::Value as TomlValue;
use zip::ZipArchive;

const MAX_METADATA_BYTES: u64 = 1024 * 1024;
const MAX_ICON_BYTES: u64 = 8 * 1024 * 1024;
const PREFERRED_ICON_SIZE: u32 = 96;
const MAX_NAME_CHARS: usize = 256;
const MAX_VERSION_CHARS: usize = 128;
const MAX_DESCRIPTION_CHARS: usize = 4096;

pub(crate) struct ArchiveInspection {
    pub metadata: Option<EmbeddedContentMetadata>,
    pub icon: Option<Bytes>,
}

#[derive(Clone, Copy)]
enum ModMetadataKind {
    Fabric,
    Quilt,
    NeoForge,
    Forge,
    LegacyForge,
}

pub(crate) async fn infer_project_type_path(
    path: &Path,
) -> crate::Result<ProjectType> {
    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || {
        let mut archive = ZipArchive::new(File::open(path)?).context(
            ErrorKind::InputError(
                "unable to infer project type for input file".to_string(),
            ),
        )?;
        infer_project_type(&mut archive)
    })
    .await?
}

pub(crate) fn infer_project_type_bytes(
    bytes: &Bytes,
) -> crate::Result<ProjectType> {
    let mut archive = ZipArchive::new(Cursor::new(&**bytes)).context(
        ErrorKind::InputError(
            "unable to infer project type for input file".to_string(),
        ),
    )?;
    infer_project_type(&mut archive)
}

fn inspect_content_file(
    path: &Path,
    loader: ModLoader,
) -> crate::Result<ArchiveInspection> {
    let file = File::open(path).context(format!(
        "could not open content archive {}",
        path.display()
    ))?;
    inspect_content_archive(file, Some(loader))
}

fn inspect_content_archive<R: Read + Seek>(
    reader: R,
    loader: Option<ModLoader>,
) -> crate::Result<ArchiveInspection> {
    let mut archive =
        ZipArchive::new(reader).context(ErrorKind::InputError(
            "unable to infer project type for input file".to_string(),
        ))?;
    let project_type = infer_project_type(&mut archive)?;
    let (metadata, icon) = match project_type {
        ProjectType::Mod => inspect_mod(&mut archive, loader),
        ProjectType::DataPack | ProjectType::ResourcePack => {
            inspect_pack(&mut archive)
        }
        ProjectType::ShaderPack => (None, None),
    };

    Ok(ArchiveInspection { metadata, icon })
}

fn infer_project_type<R: Read + Seek>(
    archive: &mut ZipArchive<R>,
) -> crate::Result<ProjectType> {
    if has_entry(archive, "fabric.mod.json")
        || has_entry(archive, "quilt.mod.json")
        || has_entry(archive, "META-INF/neoforge.mods.toml")
        || has_entry(archive, "META-INF/mods.toml")
        || has_entry(archive, "mcmod.info")
    {
        Ok(ProjectType::Mod)
    } else if has_entry(archive, "pack.mcmeta") {
        if archive.file_names().any(|name| name.starts_with("data/")) {
            Ok(ProjectType::DataPack)
        } else {
            Ok(ProjectType::ResourcePack)
        }
    } else if archive
        .file_names()
        .any(|name| name.starts_with("shaders/"))
    {
        Ok(ProjectType::ShaderPack)
    } else {
        Err(ErrorKind::InputError(
            "unable to infer project type for input file".to_string(),
        )
        .into())
    }
}

fn inspect_pack<R: Read + Seek>(
    archive: &mut ZipArchive<R>,
) -> (Option<EmbeddedContentMetadata>, Option<Bytes>) {
    let icon = read_icon_entry(archive, "pack.png");
    let description = read_json_entry(archive, "pack.mcmeta").and_then(|root| {
        root.get("pack")
            .and_then(|pack| pack.get("description"))
            .and_then(json_plain_text)
    });
    let metadata = EmbeddedContentMetadata {
        description,
        ..Default::default()
    };

    if metadata.is_empty() && icon.is_none() {
        (None, None)
    } else {
        (Some(metadata), icon)
    }
}

fn inspect_mod<R: Read + Seek>(
    archive: &mut ZipArchive<R>,
    loader: Option<ModLoader>,
) -> (Option<EmbeddedContentMetadata>, Option<Bytes>) {
    let manifest = read_manifest(archive);
    let order = metadata_order(loader);
    let parsed = order.iter().find_map(|kind| match kind {
        ModMetadataKind::Fabric => parse_fabric_metadata(archive),
        ModMetadataKind::Quilt => parse_quilt_metadata(archive),
        ModMetadataKind::NeoForge => parse_toml_metadata(
            archive,
            "META-INF/neoforge.mods.toml",
            &manifest,
        ),
        ModMetadataKind::Forge => {
            parse_toml_metadata(archive, "META-INF/mods.toml", &manifest)
        }
        ModMetadataKind::LegacyForge => parse_legacy_forge_metadata(archive),
    });
    let (mut metadata, icon_path) =
        parsed.unwrap_or_else(|| (EmbeddedContentMetadata::default(), None));

    if metadata.name.is_none() {
        metadata.name = manifest
            .get("implementation-title")
            .and_then(|value| clean_string(value, MAX_NAME_CHARS));
    }
    if metadata.version.is_none() {
        metadata.version = manifest
            .get("implementation-version")
            .and_then(|value| clean_string(value, MAX_VERSION_CHARS));
    }

    let icon = icon_path
        .as_deref()
        .and_then(|path| read_icon_entry(archive, path));
    if metadata.is_empty() && icon.is_none() {
        (None, None)
    } else {
        (Some(metadata), icon)
    }
}

fn metadata_order(loader: Option<ModLoader>) -> [ModMetadataKind; 5] {
    match loader {
        Some(ModLoader::Fabric) => [
            ModMetadataKind::Fabric,
            ModMetadataKind::Quilt,
            ModMetadataKind::NeoForge,
            ModMetadataKind::Forge,
            ModMetadataKind::LegacyForge,
        ],
        Some(ModLoader::Quilt) => [
            ModMetadataKind::Quilt,
            ModMetadataKind::Fabric,
            ModMetadataKind::NeoForge,
            ModMetadataKind::Forge,
            ModMetadataKind::LegacyForge,
        ],
        Some(ModLoader::Forge) => [
            ModMetadataKind::Forge,
            ModMetadataKind::LegacyForge,
            ModMetadataKind::NeoForge,
            ModMetadataKind::Fabric,
            ModMetadataKind::Quilt,
        ],
        Some(ModLoader::NeoForge) => [
            ModMetadataKind::NeoForge,
            ModMetadataKind::Forge,
            ModMetadataKind::LegacyForge,
            ModMetadataKind::Fabric,
            ModMetadataKind::Quilt,
        ],
        Some(ModLoader::Vanilla) | None => [
            ModMetadataKind::Fabric,
            ModMetadataKind::Quilt,
            ModMetadataKind::NeoForge,
            ModMetadataKind::Forge,
            ModMetadataKind::LegacyForge,
        ],
    }
}

fn parse_fabric_metadata<R: Read + Seek>(
    archive: &mut ZipArchive<R>,
) -> Option<(EmbeddedContentMetadata, Option<String>)> {
    let root = read_json_entry(archive, "fabric.mod.json")?;
    let metadata = EmbeddedContentMetadata {
        name: root
            .get("name")
            .and_then(JsonValue::as_str)
            .or_else(|| root.get("id").and_then(JsonValue::as_str))
            .and_then(|value| clean_string(value, MAX_NAME_CHARS)),
        version: root
            .get("version")
            .and_then(json_scalar_string)
            .and_then(|value| clean_string(&value, MAX_VERSION_CHARS)),
        description: root
            .get("description")
            .and_then(JsonValue::as_str)
            .and_then(|value| clean_string(value, MAX_DESCRIPTION_CHARS)),
        mod_id: root
            .get("id")
            .and_then(JsonValue::as_str)
            .and_then(|value| clean_string(value, MAX_NAME_CHARS)),
        authors: json_authors(root.get("authors")),
        ..Default::default()
    };
    let icon = root.get("icon").and_then(json_icon_path);
    Some((metadata, icon))
}

fn parse_quilt_metadata<R: Read + Seek>(
    archive: &mut ZipArchive<R>,
) -> Option<(EmbeddedContentMetadata, Option<String>)> {
    let text = read_text_entry(archive, "quilt.mod.json")?;
    let root = json5::from_str::<JsonValue>(&text).ok()?;
    let loader = root.get("quilt_loader").unwrap_or(&root);
    let display = loader.get("metadata").unwrap_or(loader);
    let metadata = EmbeddedContentMetadata {
        name: display
            .get("name")
            .and_then(JsonValue::as_str)
            .or_else(|| loader.get("id").and_then(JsonValue::as_str))
            .and_then(|value| clean_string(value, MAX_NAME_CHARS)),
        version: loader
            .get("version")
            .and_then(json_scalar_string)
            .and_then(|value| clean_string(&value, MAX_VERSION_CHARS)),
        description: display
            .get("description")
            .and_then(JsonValue::as_str)
            .and_then(|value| clean_string(value, MAX_DESCRIPTION_CHARS)),
        mod_id: loader
            .get("id")
            .and_then(JsonValue::as_str)
            .and_then(|value| clean_string(value, MAX_NAME_CHARS)),
        authors: json_authors(display.get("contributors").or_else(|| display.get("authors"))),
        ..Default::default()
    };
    let icon = display
        .get("icon")
        .or_else(|| loader.get("icon"))
        .and_then(json_icon_path);
    Some((metadata, icon))
}

fn parse_toml_metadata<R: Read + Seek>(
    archive: &mut ZipArchive<R>,
    path: &str,
    manifest: &HashMap<String, String>,
) -> Option<(EmbeddedContentMetadata, Option<String>)> {
    let text = read_text_entry(archive, path)?;
    let root = toml::from_str::<TomlValue>(&text).ok()?;
    let mod_table = root.get("mods")?.as_array()?.first()?.as_table()?;
    let name = mod_table
        .get("displayName")
        .or_else(|| mod_table.get("modId"))
        .and_then(toml_scalar_string)
        .and_then(|value| clean_string(&value, MAX_NAME_CHARS));
    let version = mod_table
        .get("version")
        .and_then(toml_scalar_string)
        .and_then(|value| resolve_toml_value(&value, &root, manifest))
        .and_then(|value| clean_string(&value, MAX_VERSION_CHARS));
    let icon = mod_table
        .get("logoFile")
        .and_then(TomlValue::as_str)
        .and_then(safe_archive_path);

    Some((
        EmbeddedContentMetadata {
            name,
            version,
            description: mod_table
                .get("description")
                .and_then(TomlValue::as_str)
                .and_then(|value| clean_string(value, MAX_DESCRIPTION_CHARS)),
            mod_id: mod_table
                .get("modId")
                .and_then(TomlValue::as_str)
                .and_then(|value| clean_string(value, MAX_NAME_CHARS)),
            authors: mod_table
                .get("authors")
                .and_then(toml_scalar_string)
                .and_then(|value| clean_string(&value, MAX_METADATA_BYTES as usize))
                .map(|value| vec![value]),
            ..Default::default()
        },
        icon,
    ))
}

fn parse_legacy_forge_metadata<R: Read + Seek>(
    archive: &mut ZipArchive<R>,
) -> Option<(EmbeddedContentMetadata, Option<String>)> {
    let root = read_json_entry(archive, "mcmod.info")?;
    let entry = root
        .as_array()
        .and_then(|entries| entries.first())
        .or_else(|| {
            root.get("modList")
                .and_then(JsonValue::as_array)
                .and_then(|entries| entries.first())
        })?;
    let metadata = EmbeddedContentMetadata {
        name: entry
            .get("name")
            .and_then(JsonValue::as_str)
            .or_else(|| entry.get("modid").and_then(JsonValue::as_str))
            .and_then(|value| clean_string(value, MAX_NAME_CHARS)),
        version: entry
            .get("version")
            .and_then(json_scalar_string)
            .and_then(|value| clean_string(&value, MAX_VERSION_CHARS)),
        description: entry
            .get("description")
            .and_then(JsonValue::as_str)
            .and_then(|value| clean_string(value, MAX_DESCRIPTION_CHARS)),
        mod_id: entry
            .get("modid")
            .and_then(JsonValue::as_str)
            .and_then(|value| clean_string(value, MAX_NAME_CHARS)),
        authors: json_authors(entry.get("authorList").or_else(|| entry.get("authors"))),
        ..Default::default()
    };
    let icon = entry
        .get("logoFile")
        .and_then(JsonValue::as_str)
        .and_then(safe_archive_path);
    Some((metadata, icon))
}

fn resolve_toml_value(
    value: &str,
    root: &TomlValue,
    manifest: &HashMap<String, String>,
) -> Option<String> {
    if value == "${file.jarVersion}" {
        return manifest.get("implementation-version").cloned();
    }
    if let Some(key) = value
        .strip_prefix("${file.")
        .and_then(|value| value.strip_suffix('}'))
    {
        return root
            .get("properties")
            .and_then(TomlValue::as_table)
            .and_then(|properties| properties.get(key))
            .and_then(toml_scalar_string);
    }
    Some(value.to_string())
}

fn json_icon_path(value: &JsonValue) -> Option<String> {
    if let Some(path) = value.as_str() {
        return safe_archive_path(path);
    }
    let icons = value.as_object()?;
    let mut candidates = icons
        .iter()
        .filter_map(|(size, path)| {
            Some((
                size.parse::<u32>().ok()?,
                safe_archive_path(path.as_str()?)?,
            ))
        })
        .collect::<Vec<_>>();
    candidates.sort_by_key(|(size, _)| *size);
    candidates
        .iter()
        .find(|(size, _)| *size >= PREFERRED_ICON_SIZE)
        .or_else(|| candidates.last())
        .map(|(_, path)| path.clone())
}

fn safe_archive_path(path: &str) -> Option<String> {
    let path = path.trim().trim_start_matches('/');
    if path.is_empty()
        || path.contains('\0')
        || path.contains('\\')
        || path
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return None;
    }
    Some(path.to_string())
}

fn clean_string(value: &str, max_chars: usize) -> Option<String> {
    let value = value.trim();
    if value.is_empty() || value.contains("${") {
        return None;
    }
    let value = value
        .chars()
        .filter(|character| !character.is_control())
        .take(max_chars)
        .collect::<String>();
    (!value.is_empty()).then_some(value)
}

fn json_scalar_string(value: &JsonValue) -> Option<String> {
    match value {
        JsonValue::String(value) => Some(value.clone()),
        JsonValue::Number(value) => Some(value.to_string()),
        _ => None,
    }
}

fn json_authors(value: Option<&JsonValue>) -> Option<Vec<String>> {
	let authors: Vec<String> = match value? {
		JsonValue::String(author) => clean_string(author, MAX_NAME_CHARS)
			.into_iter()
			.collect(),
		JsonValue::Object(authors) => authors
			.keys()
			.filter_map(|author| clean_string(author, MAX_NAME_CHARS))
			.take(32)
			.collect(),
		JsonValue::Array(authors) => authors
			.iter()
			.filter_map(|author| {
				author
					.as_str()
					.or_else(|| author.get("name").and_then(JsonValue::as_str))
					.and_then(|author| clean_string(author, MAX_NAME_CHARS))
			})
			.take(32)
			.collect(),
		_ => return None,
	};
	(!authors.is_empty()).then_some(authors)
}

fn json_plain_text(value: &JsonValue) -> Option<String> {
	fn append(value: &JsonValue, output: &mut String) {
		match value {
			JsonValue::String(text) => output.push_str(text),
			JsonValue::Array(parts) => {
				for part in parts {
					append(part, output);
				}
			}
			JsonValue::Object(component) => {
				if let Some(text) = component.get("text") {
					append(text, output);
				}
				if let Some(extra) = component.get("extra") {
					append(extra, output);
				}
			}
			_ => {}
		}
	}
	let mut output = String::new();
	append(value, &mut output);
	clean_string(&output, MAX_DESCRIPTION_CHARS)
}

fn toml_scalar_string(value: &TomlValue) -> Option<String> {
    match value {
        TomlValue::String(value) => Some(value.clone()),
        TomlValue::Integer(value) => Some(value.to_string()),
        TomlValue::Float(value) => Some(value.to_string()),
        _ => None,
    }
}

fn read_manifest<R: Read + Seek>(
    archive: &mut ZipArchive<R>,
) -> HashMap<String, String> {
    let Some(text) = read_text_entry(archive, "META-INF/MANIFEST.MF") else {
        return HashMap::new();
    };
    let mut attributes: HashMap<String, String> = HashMap::new();
    let mut current_key: Option<String> = None;
    for line in text.lines() {
        if let Some(continuation) = line.strip_prefix(' ') {
            if let Some(key) = current_key.as_ref()
                && let Some(value) = attributes.get_mut(key)
            {
                value.push_str(continuation);
            }
            continue;
        }
        let Some((key, value)) = line.split_once(':') else {
            current_key = None;
            continue;
        };
        let key = key.trim().to_ascii_lowercase();
        attributes.insert(key.clone(), value.trim_start().to_string());
        current_key = Some(key);
    }
    attributes
}

fn has_entry<R: Read + Seek>(archive: &mut ZipArchive<R>, path: &str) -> bool {
    archive.by_name(path).is_ok()
}

fn read_json_entry<R: Read + Seek>(
    archive: &mut ZipArchive<R>,
    path: &str,
) -> Option<JsonValue> {
    let text = read_text_entry(archive, path)?;
    serde_json::from_str(&text).ok()
}

fn read_text_entry<R: Read + Seek>(
    archive: &mut ZipArchive<R>,
    path: &str,
) -> Option<String> {
    let bytes = read_entry(archive, path, MAX_METADATA_BYTES)?;
    String::from_utf8(bytes).ok()
}

fn read_icon_entry<R: Read + Seek>(
    archive: &mut ZipArchive<R>,
    path: &str,
) -> Option<Bytes> {
    let path = safe_archive_path(path)?;
    read_entry(archive, &path, MAX_ICON_BYTES).map(Bytes::from)
}

fn read_entry<R: Read + Seek>(
    archive: &mut ZipArchive<R>,
    path: &str,
    max_bytes: u64,
) -> Option<Vec<u8>> {
    let entry = archive.by_name(path).ok()?;
    if entry.size() > max_bytes {
        return None;
    }
    let mut bytes = Vec::with_capacity(entry.size() as usize);
    entry
        .take(max_bytes.saturating_add(1))
        .read_to_end(&mut bytes)
        .ok()?;
    (bytes.len() as u64 <= max_bytes).then_some(bytes)
}

pub(crate) async fn resolve_embedded_content_metadata(
    instance: &Instance,
    loader: ModLoader,
    files: &[(String, ContentFile)],
    state: &State,
) -> crate::Result<HashMap<String, EmbeddedContentMetadata>> {
    let tracked = crate::state::instances::adapters::sqlite::content_rows::get_instance_files(&instance.id, &state.pool).await?;
    let mut candidates = HashMap::new();
    for (relative_path, file) in files {
        if !matches!(
                file.project_type,
                ProjectType::Mod
                    | ProjectType::DataPack
                    | ProjectType::ResourcePack
                    | ProjectType::ShaderPack
            )
        {
            continue;
        }
        let Some(tracked) = tracked
            .iter()
            .find(|tracked| &tracked.relative_path == relative_path)
        else {
            continue;
        };
        if let Ok(path) =
            state.content_store.read_path(tracked, &instance.path).await
        {
            candidates.insert(file.hash.clone(), path);
        }
    }
    if candidates.is_empty() {
        return Ok(HashMap::new());
    }

    let cache_keys = candidates
        .keys()
        .map(|hash| metadata_cache_key(hash, loader))
        .collect::<Vec<_>>();
    let cache_key_refs =
        cache_keys.iter().map(String::as_str).collect::<Vec<_>>();
    let cached = CachedEntry::get_embedded_content_metadata_many(
        &cache_key_refs,
        None,
        &state.pool,
        &state.api_semaphore,
    )
    .await?;
    let mut resolved = HashMap::new();
    let mut resolved_hashes = HashSet::new();
    for cached in cached {
        let icon_exists = cached
            .metadata
            .as_ref()
            .and_then(|metadata| metadata.icon_path.as_deref())
            .is_none_or(|path| Path::new(path).is_file());
        if icon_exists {
            resolved_hashes.insert(cached.hash.clone());
            if let Some(metadata) = cached.metadata {
                resolved.insert(cached.hash, metadata);
            }
        }
    }

    let pending = candidates
        .into_iter()
        .filter(|(hash, _)| !resolved_hashes.contains(hash))
        .collect::<Vec<_>>();
    let inspected_metadata = stream::iter(pending)
        .map(|(hash, path)| async move {
            let inspection = tokio::task::spawn_blocking(move || {
                inspect_content_file(path.path(), loader)
            })
            .await;
            let (mut metadata, icon) = match inspection {
                Ok(Ok(inspection)) => {
                    (inspection.metadata.unwrap_or_default(), inspection.icon)
                }
                Ok(Err(error)) => {
                    tracing::debug!(
                        hash,
                        error = %error,
                        "Could not inspect content metadata"
                    );
                    return None;
                }
                Err(error) => {
                    tracing::debug!(
                        hash,
                        error = %error,
                        "Content metadata inspection task failed"
                    );
                    return None;
                }
            };
            if let Some(icon) = icon {
                match crate::api::instance::cache_icon(icon, state).await {
                    Ok(path) => {
                        metadata.icon_path =
                            Some(path.to_string_lossy().to_string());
                    }
                    Err(error) => {
                        tracing::debug!(
                            hash,
                            error = %error,
                            "Could not cache embedded content icon"
                        );
                    }
                }
            }
            let metadata = (!metadata.is_empty()).then_some(metadata);
            Some((hash, metadata))
        })
        .buffer_unordered(8)
        .collect::<Vec<_>>()
        .await;
    let mut entries = Vec::with_capacity(inspected_metadata.len());
    for (hash, metadata) in inspected_metadata.into_iter().flatten() {
        if let Some(metadata) = metadata.as_ref() {
            resolved.insert(hash.clone(), metadata.clone());
        }
        entries.push(
            CacheValue::EmbeddedContentMetadata(
                CachedEmbeddedContentMetadata {
                    cache_key: metadata_cache_key(&hash, loader),
                    hash,
                    metadata,
                },
            )
            .get_entry(),
        );
    }
    CachedEntry::upsert_many(&entries, &state.pool).await?;

    Ok(resolved)
}

fn metadata_cache_key(hash: &str, loader: ModLoader) -> String {
    format!("{hash}-{}-v2", loader.as_str())
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::io::{Cursor, Write};
	use zip::write::SimpleFileOptions;

	fn archive(files: &[(&str, &str)]) -> Cursor<Vec<u8>> {
		let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
		for (path, body) in files {
			writer.start_file(*path, SimpleFileOptions::default()).unwrap();
			writer.write_all(body.as_bytes()).unwrap();
		}
		writer.finish().unwrap()
	}

	#[test]
	fn parses_fabric_metadata_fields() {
		let mut zip = ZipArchive::new(archive(&[("fabric.mod.json", r#"{
			"id":"example_mod","name":"Example","version":"2.3",
			"description":"A local mod","authors":["One",{"name":"Two"}]
		}"#)])).unwrap();
		let (metadata, _) = parse_fabric_metadata(&mut zip).unwrap();
		assert_eq!(metadata.mod_id.as_deref(), Some("example_mod"));
		assert_eq!(metadata.description.as_deref(), Some("A local mod"));
		assert_eq!(metadata.authors, Some(vec!["One".into(), "Two".into()]));
	}

	#[test]
	fn reads_resource_pack_component_description() {
		let mut zip = ZipArchive::new(archive(&[("pack.mcmeta", r#"{
			"pack":{"description":{"text":"Pack ","extra":[{"text":"description"}]}}
		}"#)])).unwrap();
		let (metadata, _) = inspect_pack(&mut zip);
		assert_eq!(metadata.unwrap().description.as_deref(), Some("Pack description"));
	}

	#[test]
	fn parses_quilt_metadata_and_contributors() {
		let mut zip = ZipArchive::new(archive(&[("quilt.mod.json", r#"{
			quilt_loader: { id: 'quilt_sample', version: '1.2', metadata: {
				name: 'Quilt Sample', description: 'Quilt local metadata',
				contributors: { 'Quilt Author': 'Owner' }
			} }
		}"#)])).unwrap();
		let (metadata, _) = parse_quilt_metadata(&mut zip).unwrap();
		assert_eq!(metadata.mod_id.as_deref(), Some("quilt_sample"));
		assert_eq!(metadata.description.as_deref(), Some("Quilt local metadata"));
		assert_eq!(metadata.authors, Some(vec!["Quilt Author".to_string()]));
	}

	#[test]
	fn parses_forge_metadata_fields() {
		let body = "[[mods]]\nmodId = 'forge_sample'\ndisplayName = 'Forge Sample'\nversion = '1.0'\ndescription = 'Forge local metadata'\nauthors = 'Forge Author'\n";
		let mut zip = ZipArchive::new(archive(&[("META-INF/mods.toml", body)])).unwrap();
		let (metadata, _) = parse_toml_metadata(&mut zip, "META-INF/mods.toml", &HashMap::new()).unwrap();
		assert_eq!(metadata.mod_id.as_deref(), Some("forge_sample"));
		assert_eq!(metadata.description.as_deref(), Some("Forge local metadata"));
		assert_eq!(metadata.authors, Some(vec!["Forge Author".to_string()]));
	}

	#[test]
	fn parses_legacy_forge_metadata_fields() {
		let mut zip = ZipArchive::new(archive(&[("mcmod.info", r#"[{"modid":"old_sample","name":"Old Sample","version":"2","description":"Legacy local metadata","authorList":["Old Author"]}]"#)])).unwrap();
		let (metadata, _) = parse_legacy_forge_metadata(&mut zip).unwrap();
		assert_eq!(metadata.mod_id.as_deref(), Some("old_sample"));
		assert_eq!(metadata.description.as_deref(), Some("Legacy local metadata"));
		assert_eq!(metadata.authors, Some(vec!["Old Author".to_string()]));
	}

	#[test]
	fn malformed_and_oversized_manifests_are_ignored() {
		let mut malformed = ZipArchive::new(archive(&[("fabric.mod.json", "{bad json")])).unwrap();
		assert!(parse_fabric_metadata(&mut malformed).is_none());
		let oversized = "x".repeat(MAX_METADATA_BYTES as usize + 1);
		let mut large = ZipArchive::new(archive(&[("fabric.mod.json", &oversized)])).unwrap();
		assert!(parse_fabric_metadata(&mut large).is_none());
	}

	#[test]
	fn ignores_malformed_and_oversized_description_values() {
		assert_eq!(json_plain_text(&JsonValue::Bool(true)), None);
		let long = "x".repeat(MAX_DESCRIPTION_CHARS + 100);
		let value = json_plain_text(&JsonValue::String(long)).unwrap();
		assert_eq!(value.chars().count(), MAX_DESCRIPTION_CHARS);
		let author = JsonValue::String("x".repeat(400));
		assert_eq!(json_authors(Some(&author)).unwrap()[0].len(), MAX_NAME_CHARS);
	}

	#[test]
	fn metadata_cache_key_is_versioned() {
		assert!(metadata_cache_key("abc", ModLoader::Fabric).ends_with("-v2"));
	}
}
