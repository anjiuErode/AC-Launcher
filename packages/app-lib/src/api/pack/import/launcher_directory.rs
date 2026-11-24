use std::{
    collections::BTreeSet,
    fs::{self, File, Metadata},
    io::{self, Read},
    path::{Component, Path, PathBuf},
};

use anyhow::{Context, bail, ensure};
use serde::{Deserialize, Serialize};

use crate::state::ModLoader;

use super::external::InstanceSpec;

const GAME_DIRS: &[&str] = &[
    "saves", "mods", "config", "defaultconfigs", "resourcepacks", "shaderpacks",
    "datapacks", "screenshots", "schematics", "kubejs", "scripts",
];
const GAME_FILES: &[&str] = &[
    "options.txt", "optionsof.txt", "optionsshaders.txt", "servers.dat", "hotbar.nbt",
];

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum LauncherDirectoryType {
    Pcl2,
    PclCe,
    Hmcl,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum FolderLayout {
    Shared,
    Isolated,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LauncherVersionPreview {
    pub id: String,
    pub spec: Option<InstanceSpec>,
    pub error: Option<String>,
    pub suggested_layout: Option<FolderLayout>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LauncherContentPreview {
    pub source_path: PathBuf,
    pub saves: usize,
    pub mods: usize,
    pub config_files: usize,
    pub missing_paths: Vec<&'static str>,
}

struct LauncherRoot {
    root: PathBuf,
    selected_version: Option<String>,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct VersionMetadata {
    id: Option<String>,
    version: Option<String>,
    inherits_from: Option<String>,
    client_version: Option<String>,
    jar: Option<String>,
    main_class: Option<String>,
    #[serde(default)]
    libraries: Vec<VersionLibrary>,
    #[serde(default)]
    patches: Vec<VersionMetadata>,
}

#[derive(Deserialize)]
struct VersionLibrary {
    name: String,
}

pub async fn inspect_versions(
    path: PathBuf,
    launcher: LauncherDirectoryType,
) -> crate::Result<Vec<LauncherVersionPreview>> {
    Ok(tokio::task::spawn_blocking(move || {
        let root = launcher_root(&path)?;
        let mut output = Vec::new();
        for entry in fs::read_dir(root.root.join("versions"))? {
            let entry = entry?;
            let Some(id) = entry.file_name().into_string().ok() else {
                continue;
            };
            if root.selected_version.as_ref().is_some_and(|name| name != &id)
                || !entry.file_type()?.is_dir()
            {
                continue;
            }
            let result = version_spec(&root.root, &id)
                .and_then(|spec| Ok((spec, infer_layout(&entry.path(), launcher)?)));
            let (spec, suggested_layout, error) = match result {
                Ok((spec, layout)) => (Some(spec), layout, None),
                Err(error) => (None, None, Some(error.to_string())),
            };
            output.push(LauncherVersionPreview { id, spec, error, suggested_layout });
        }
        output.sort_by(|a, b| a.id.cmp(&b.id));
        ensure!(!output.is_empty(), "No Java Edition versions found in the selected directory.");
        Ok::<_, anyhow::Error>(output)
    }).await??)
}

pub async fn inspect_content(
    path: PathBuf,
    version_id: String,
    layout: FolderLayout,
) -> crate::Result<LauncherContentPreview> {
    Ok(tokio::task::spawn_blocking(move || {
        let (_, source) = resolve_source(&path, &version_id, layout)?;
        let mut preview = LauncherContentPreview {
            source_path: dunce::simplified(&source).to_path_buf(),
            saves: 0,
            mods: 0,
            config_files: 0,
            missing_paths: Vec::new(),
        };
        for name in ["saves", "mods", "config", "options.txt"] {
            let path = source.join(name);
            if metadata_if_present(&path)?.is_none() {
                preview.missing_paths.push(name);
                continue;
            }
            match name {
                "saves" => preview.saves = fs::read_dir(&path)?.try_fold(0, |count, entry| {
                    let metadata = checked_metadata(&entry?.path())?;
                    Ok::<_, anyhow::Error>(count + usize::from(metadata.is_dir()))
                })?,
                "mods" => preview.mods = fs::read_dir(&path)?.try_fold(0, |count, entry| {
                    let metadata = checked_metadata(&entry?.path())?;
                    Ok::<_, anyhow::Error>(count + usize::from(metadata.is_file()))
                })?,
                "config" => preview.config_files = visit_files(&path, &path, &mut |_, _| Ok(()))?,
                _ => continue,
            }
        }
        Ok::<_, anyhow::Error>(preview)
    }).await??)
}

pub(super) async fn prepare(
    path: PathBuf,
    launcher: LauncherDirectoryType,
    version_id: String,
    layout: FolderLayout,
    instances: PathBuf,
    staging: PathBuf,
) -> crate::Result<InstanceSpec> {
    Ok(tokio::task::spawn_blocking(move || {
        let (root, source) = resolve_source(&path, &version_id, layout)?;
        let source = fs::canonicalize(source)?;
        let instances = fs::canonicalize(instances)?;
        ensure!(
            !source.starts_with(&instances) && !instances.starts_with(&source),
            "The game data directory overlaps the ACL instances directory. Select a separate game data directory."
        );
        let spec = version_spec(&root, &version_id)?;
        infer_layout(&root.join("versions").join(&version_id), launcher)?;
        fs::create_dir(&staging)?;
        copy_game_data(&source, &staging)?;
        Ok::<_, anyhow::Error>(spec)
    }).await??)
}

fn resolve_source(path: &Path, version_id: &str, layout: FolderLayout) -> anyhow::Result<(PathBuf, PathBuf)> {
    ensure!(is_single_name(version_id), "Invalid Minecraft version directory name.");
    let root = launcher_root(path)?;
    ensure!(root.selected_version.is_none_or(|id| id == version_id), "The selected Minecraft version changed.");
    let version = root.root.join("versions").join(version_id);
    ensure!(checked_metadata(&version)?.is_dir(), "Missing Minecraft version directory.");
    let source = match layout {
        FolderLayout::Shared => root.root.clone(),
        FolderLayout::Isolated => version,
    };
    Ok((root.root, source))
}

fn launcher_root(path: &Path) -> anyhow::Result<LauncherRoot> {
    ensure!(checked_metadata(path)?.is_dir(), "Select a launcher directory.");
    let path = fs::canonicalize(path)?;
    let parent = path.parent();
    let (root, selected_version) = if let Some(parent) = parent
        && parent.file_name().is_some_and(|part| part.eq_ignore_ascii_case("versions"))
    {
        checked_metadata(parent)?;
        (parent.parent().context("Missing Minecraft root.")?.to_path_buf(), Some(path.file_name().and_then(|name| name.to_str()).context("Invalid version name.")?.to_owned()))
    } else if path.file_name().is_some_and(|part| part.eq_ignore_ascii_case("versions")) {
        (parent.context("Missing Minecraft root.")?.to_path_buf(), None)
    } else if metadata_if_present(&path.join("versions"))?.is_some() {
        (path, None)
    } else {
        let minecraft = path.join(".minecraft");
        ensure!(checked_metadata(&minecraft)?.is_dir(), "No .minecraft directory found.");
        (minecraft, None)
    };
    ensure!(checked_metadata(&root.join("versions"))?.is_dir(), "No versions directory found.");
    Ok(LauncherRoot {root, selected_version})
}

fn version_spec(root: &Path, version_id: &str) -> anyhow::Result<InstanceSpec> {
    let mut visited = BTreeSet::new();
    let mut next = Some(version_id.to_owned());
    let mut game_version = None;
    let mut forge_game_version = None;
    let mut loader = None;
    while let Some(id) = next {
        ensure!(is_single_name(&id) && visited.insert(id.clone()) && visited.len() <= 16, "Invalid Minecraft version inheritance.");
        let directory = root.join("versions").join(&id);
        let json = directory.join(format!("{id}.json"));
        if id != version_id && metadata_if_present(&directory)?.is_none() {
            ensure!(is_game_version_id(&id), "Missing inherited Minecraft version {id}.");
            game_version.get_or_insert(id);
            break;
        }
        ensure!(checked_metadata(&directory)?.is_dir(), "Invalid Minecraft version directory {id}.");
        let bytes = read_small_file(&json).with_context(|| format!("Missing or unreadable {id}.json"))?;
        let metadata: VersionMetadata = serde_json::from_slice(bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(&bytes))
            .with_context(|| format!("Invalid launch metadata for {id}"))?;
        let mut profiles = vec![&metadata];
        profiles.extend(&metadata.patches);
        for profile in profiles {
            if profile.main_class.as_deref().is_some_and(|class| class.to_ascii_lowercase().contains("optifine")) {
                bail!("The OptiFine client must be installed separately before migration.");
            }
            for library in &profile.libraries {
                let mut parts = library.name.split(':');
                let (Some(group), Some(artifact), Some(version)) = (parts.next(), parts.next(), parts.next()) else { continue };
                let found = match (group, artifact) {
                    ("net.fabricmc", "fabric-loader") => ModLoader::Fabric,
                    ("org.quiltmc", "quilt-loader") => ModLoader::Quilt,
                    ("net.minecraftforge", "forge" | "fmlloader" | "minecraftforge") => ModLoader::Forge,
                    ("net.neoforged", "neoforge" | "forge") => ModLoader::NeoForge,
                    ("optifine" | "com.mumfrey" | "net.legacyfabric", _) => bail!("This version uses an unsupported library ({group}:{artifact})."),
                    _ => continue,
                };
                if loader.as_ref().is_some_and(|(existing, _)| *existing != found) {
                    bail!("This version contains more than one mod loader.");
                }
                if found == ModLoader::Forge {
                    if let Some((minecraft, _)) = version.split_once('-')
                        && is_game_version_id(minecraft)
                    {
                        forge_game_version.get_or_insert_with(|| minecraft.to_owned());
                    }
                }
                loader.get_or_insert((found, version.to_owned()));
            }
            for candidate in [
                profile.client_version.as_ref(),
                (profile.id.as_deref() == Some("game")).then_some(profile.version.as_ref()).flatten(),
                profile.jar.as_ref(),
            ].into_iter().flatten() {
                if is_game_version_id(candidate) {
                    game_version.get_or_insert_with(|| candidate.clone());
                }
            }
        }
        next = metadata.inherits_from;
        if next.is_none() && is_game_version_id(&id) {
            game_version.get_or_insert(id);
        }
    }
    let game_version = game_version.or(forge_game_version)
        .context("Could not identify the original Minecraft version from this profile.")?;
    let (loader, loader_version) = match loader {
        Some((loader, version)) => {
            let version = super::external::normalize_loader_version(loader, &game_version, &version);
            ensure!(version.chars().any(|character| character.is_ascii_digit()), "The original mod loader version is missing.");
            (loader, Some(version))
        }
        None => (ModLoader::Vanilla, None),
    };
    Ok(InstanceSpec { name: version_id.to_owned(), game_version, loader, loader_version })
}

fn is_game_version_id(value: &str) -> bool {
    is_single_name(value)
        && (value.split_once('.').is_some_and(|(major, rest)| {
            !major.is_empty() && major.bytes().all(|byte| byte.is_ascii_digit())
                && rest.starts_with(|c: char| c.is_ascii_digit())
                && !rest.chars().any(char::is_whitespace)
        }) || value.as_bytes().get(2) == Some(&b'w')
            && value.as_bytes()[..2].iter().all(u8::is_ascii_digit))
}

fn infer_layout(path: &Path, launcher: LauncherDirectoryType) -> anyhow::Result<Option<FolderLayout>> {
    match launcher {
        LauncherDirectoryType::Pcl2 | LauncherDirectoryType::PclCe => {
            let pcl = path.join("PCL");
            if metadata_if_present(&pcl)?.is_some()
                && metadata_if_present(&pcl.join("Setup.ini"))?.is_some()
            {
                let bytes = read_small_file(&pcl.join("Setup.ini"))?;
                let text = String::from_utf8_lossy(&bytes);
                let settings: Vec<_> = text.lines().filter_map(|line| line.split_once([':', '='])).collect();
                for (key, values) in [("VersionArgumentIndieV2", ("true", "false")), ("VersionArgumentIndie", ("1", "2"))] {
                    if let Some((_, value)) = settings.iter().find(|(name, _)| name.trim_matches(['\u{feff}', ' ']).eq_ignore_ascii_case(key)) {
                        let value = value.trim();
                        if value.eq_ignore_ascii_case(values.0) { return Ok(Some(FolderLayout::Isolated)); }
                        if value.eq_ignore_ascii_case(values.1) { return Ok(Some(FolderLayout::Shared)); }
                    }
                }
            }
        }
        LauncherDirectoryType::Hmcl => {
            if metadata_if_present(&path.join("modpack.cfg"))?.is_some() {
                return Ok(Some(FolderLayout::Isolated));
            }
            let metadata = path.join(".hmcl");
            let config = metadata.join("config");
            let current = config.join("instance-game-settings.json");
            if metadata_if_present(&metadata)?.is_some()
                && metadata_if_present(&config)?.is_some()
                && metadata_if_present(&current)?.is_some()
            {
                let settings: serde_json::Value = serde_json::from_slice(&read_small_file(&current)?)?;
                let overrides_dir = settings.get("overrideProperties").and_then(serde_json::Value::as_array).is_some_and(|properties| properties.iter().any(|property| property.as_str() == Some("runningDirectory")));
                if overrides_dir {
                    let directory = settings.get("runningDirectory").and_then(serde_json::Value::as_str).unwrap_or_default();
                    if directory.trim().is_empty() {
                        return Ok(Some(FolderLayout::Isolated));
                    }
                    let running = fs::canonicalize(directory).context("HMCL's custom game directory is unavailable.")?;
                    if running == path { return Ok(Some(FolderLayout::Isolated)); }
                    if path.parent().and_then(Path::parent) == Some(running.as_path()) { return Ok(Some(FolderLayout::Shared)); }
                    bail!("Import of a custom HMCL game directory is unsupported.");
                }
            }
            let settings = path.join("hmclversion.cfg");
            if metadata_if_present(&settings)?.is_some() {
                let settings: serde_json::Value = serde_json::from_slice(&read_small_file(&settings)?)?;
                if settings.get("usesGlobal").and_then(serde_json::Value::as_bool) != Some(true) {
                    return match settings.get("gameDirType").and_then(serde_json::Value::as_str) {
                        Some("VERSION_FOLDER") => Ok(Some(FolderLayout::Isolated)),
                        Some("ROOT_FOLDER") => Ok(Some(FolderLayout::Shared)),
                        Some("CUSTOM") => bail!("Import of a custom HMCL game directory is unsupported."),
                        _ => Ok(None),
                    };
                }
            }
        }
    }
    if metadata_if_present(&path.join("mods"))?.is_some() || metadata_if_present(&path.join("saves"))?.is_some() {
        return Ok(Some(FolderLayout::Isolated));
    }
    Ok(None)
}

fn copy_game_data(source: &Path, target: &Path) -> anyhow::Result<()> {
    for name in GAME_DIRS.iter().chain(GAME_FILES) {
        let candidate = source.join(name);
        if metadata_if_present(&candidate)?.is_none() { continue; }
        visit_files(source, &candidate, &mut |source, relative| {
            let target = target.join(relative);
            fs::create_dir_all(target.parent().context("Invalid migration destination.")?)?;
            fs::copy(source, target)?;
            Ok(())
        })?;
    }
    Ok(())
}

fn visit_files(root: &Path, path: &Path, visitor: &mut impl FnMut(&Path, &Path) -> anyhow::Result<()>) -> anyhow::Result<usize> {
    let mut pending = vec![path.to_path_buf()];
    let mut count = 0;
    while let Some(path) = pending.pop() {
        let metadata = checked_metadata(&path)?;
        if metadata.is_dir() {
            for entry in fs::read_dir(&path)? { pending.push(entry?.path()); }
        } else if metadata.is_file() {
            if path.file_name().is_some_and(|name| name.eq_ignore_ascii_case("session.lock")) { continue; }
            visitor(&path, path.strip_prefix(root)?)?;
            count += 1;
        } else {
            bail!("Unsupported file in migration source: {}", path.display());
        }
    }
    Ok(count)
}

pub(super) fn is_single_name(value: &str) -> bool {
    !value.is_empty() && !value.contains(['/', '\\', ':', '\0']) && value != "." && value != ".."
        && matches!(Path::new(value).components().next(), Some(Component::Normal(_)))
}

pub(super) fn checked_metadata(path: &Path) -> anyhow::Result<Metadata> {
    let metadata = fs::symlink_metadata(path)?;
    ensure!(!crate::api::instance::archive_io::path_is_link(&metadata), "Linked paths are unsupported: {}", path.display());
    Ok(metadata)
}

pub(super) fn metadata_if_present(path: &Path) -> anyhow::Result<Option<Metadata>> {
    match checked_metadata(path) {
        Ok(metadata) => Ok(Some(metadata)),
        Err(error) if error.downcast_ref::<io::Error>().is_some_and(|error| error.kind() == io::ErrorKind::NotFound) => Ok(None),
        Err(error) => Err(error),
    }
}

fn read_small_file(path: &Path) -> anyhow::Result<Vec<u8>> {
    const MAX_BYTES: u64 = 4 * 1024 * 1024;
    let metadata = checked_metadata(path)?;
    ensure!(metadata.is_file() && metadata.len() <= MAX_BYTES, "Unsupported metadata file: {}", path.display());
    let mut bytes = Vec::new();
    File::open(path)?.take(MAX_BYTES + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() as u64 <= MAX_BYTES, "Metadata file is too large: {}", path.display());
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migration_reads_selected_layout_and_preserves_source() -> anyhow::Result<()> {
        let scratch = tempfile::tempdir()?;
        let root = scratch.path().join(".minecraft");
        let version = root.join("versions/instance");
        fs::create_dir_all(version.join("mods"))?;
        fs::create_dir_all(root.join("saves/world"))?;
        fs::write(root.join("launcher_accounts.json"), "source only")?;
        fs::write(root.join("saves/world/level.dat"), b"world")?;
        fs::write(version.join("mods/mod.jar"), b"mod")?;
        fs::write(version.join("instance.json"), r#"{"inheritsFrom":"1.20.1","libraries":[{"name":"net.fabricmc:fabric-loader:0.15.11"}]}"#)?;
        let spec = version_spec(&root, "instance")?;
        assert_eq!(spec.game_version, "1.20.1");
        assert_eq!(spec.loader, ModLoader::Fabric);
        let shared = scratch.path().join("shared");
        let isolated = scratch.path().join("isolated");
        copy_game_data(&resolve_source(&root, "instance", FolderLayout::Shared)?.1, &shared)?;
        copy_game_data(&resolve_source(&version, "instance", FolderLayout::Isolated)?.1, &isolated)?;
        assert_eq!(fs::read(shared.join("saves/world/level.dat"))?, b"world");
        assert_eq!(fs::read(isolated.join("mods/mod.jar"))?, b"mod");
        assert!(!shared.join("versions").exists());
        assert!(!shared.join("launcher_accounts.json").exists());
        assert!(!isolated.join("saves").exists());
        assert_eq!(fs::read(version.join("mods/mod.jar"))?, b"mod");
        assert_eq!(fs::read(root.join("saves/world/level.dat"))?, b"world");
        Ok(())
    }

    #[test]
    fn pcli_and_hmcl_layout_metadata_is_recognized() -> anyhow::Result<()> {
        let scratch = tempfile::tempdir()?;
        let version = scratch.path();
        fs::create_dir(version.join("PCL"))?;
        fs::write(version.join("PCL/Setup.ini"), "VersionArgumentIndie:1\nVersionArgumentIndieV2:False\n")?;
        assert_eq!(infer_layout(version, LauncherDirectoryType::PclCe)?, Some(FolderLayout::Shared));
        fs::create_dir_all(version.join(".hmcl/config"))?;
        fs::write(version.join(".hmcl/config/instance-game-settings.json"), r#"{"overrideProperties":["runningDirectory"],"runningDirectory":""}"#)?;
        assert_eq!(infer_layout(version, LauncherDirectoryType::Hmcl)?, Some(FolderLayout::Isolated));
        Ok(())
    }

    #[tokio::test]
    async fn isolated_migration_allows_acl_instances_under_the_same_root() -> anyhow::Result<()> {
        let scratch = tempfile::tempdir()?;
        let root = scratch.path();
        let versions = root.join("versions");
        let source = versions.join("isolated-pack");
        let instances = root.join("ACL/profiles");
        let staging = root.join("ACL/payload");
        fs::create_dir_all(source.join("mods"))?;
        fs::create_dir_all(&instances)?;
        fs::write(
            source.join("isolated-pack.json"),
            r#"{"clientVersion":"26.2","libraries":[{"name":"net.fabricmc:fabric-loader:0.19.5"}]}"#,
        )?;
        fs::write(source.join("mods/example.jar"), b"fixture")?;

        let spec = prepare(
            source.clone(),
            LauncherDirectoryType::Pcl2,
            "isolated-pack".into(),
            FolderLayout::Isolated,
            instances.clone(),
            staging.clone(),
        ).await?;
        assert_eq!(spec.game_version, "26.2");
        assert_eq!(fs::read(staging.join("mods/example.jar"))?, b"fixture");
        assert_eq!(fs::read(source.join("mods/example.jar"))?, b"fixture");

        for (layout, instances) in [(FolderLayout::Shared, instances), (FolderLayout::Isolated, versions)] {
            let staging = root.join("rejected-payload");
            let result = prepare(
                source.clone(),
                LauncherDirectoryType::Pcl2,
                "isolated-pack".into(),
                layout,
                instances,
                staging.clone(),
            ).await;
            let Err(error) = result else { panic!("overlapping game data must be rejected") };
            assert!(format!("{error:#}").contains("game data directory overlaps"));
            assert!(!staging.exists());
        }
        Ok(())
    }
}
