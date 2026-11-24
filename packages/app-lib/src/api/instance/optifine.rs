use crate::event::InstancePayloadType;
use crate::event::emit::emit_instance;
use crate::state::instances::commands::{
    InstallContent, install_stored_file_if_absent,
};
use crate::state::{
    ContentSourceKind, DirectoryInfo, InstanceInstallStage, InstanceLink,
    InstanceMetadata, ModLoader, ProjectType, State,
};
use crate::util::fetch::sha1_file_async;
use crate::{ErrorKind, Result};
use anyhow::{Context, bail, ensure};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::io::{Cursor, ErrorKind as IoErrorKind, Read};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::fs;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;
use zip::ZipArchive;

const PATCHED_JAR: &str = "OptiFine.jar";
const WRAPPER_JAR: &str = "launchwrapper-of-2.3.jar";
const TWEAKER_CLASS: &str = "optifine/OptiFineTweaker.class";
pub(crate) const LAUNCH_MAIN: &str = "net.minecraft.launchwrapper.Launch";
pub(crate) const LAUNCH_ARGUMENTS: [&str; 2] =
    ["--tweakClass", "optifine.OptiFineTweaker"];

struct Release {
    game: &'static str,
    label: &'static str,
    forge: &'static str,
    java: u32,
    installer: &'static str,
    size: u64,
    sha256: &'static str,
    client_sha1: &'static str,
}

// Hashes are from the official OptiFine downloads and Mojang client manifests.
const RELEASES: [Release; 2] = [
    Release {
        game: "1.20.1",
        label: "1.20.1 HD U I6",
        forge: "47.2.18",
        java: 17,
        installer: "OptiFine_1.20.1_HD_U_I6.jar",
        size: 7_145_205,
        sha256: "0b67cb670aedf2e55a982f3d52b6d53e46791a0c984aa1d2ee58100fc9bfc650",
        client_sha1: "0c3ec587af28e5a785c0b4a7b8a30f9a8f78f838",
    },
    Release {
        game: "1.21.1",
        label: "1.21.1 HD U J1",
        forge: "52.0.16",
        java: 21,
        installer: "OptiFine_1.21.1_HD_U_J1.jar",
        size: 7_322_249,
        sha256: "db6d2d14db0009bdea2d8f848e5cb55aac52b555e8e50506dbf7876755b79082",
        client_sha1: "30c73b1c5da787909b2f73340419fdf13b9def88",
    },
];

#[derive(Serialize)]
pub struct OptiFineState {
    status: OptiFineStatus,
    release: Option<&'static str>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum OptiFineStatus {
    Available,
    Installed,
    Disabled,
    Conflict,
    RepairNeeded,
    Unavailable,
    Unsupported,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum InstallationMode {
    Vanilla,
    Forge,
}

fn mode(
    game: &str,
    loader: ModLoader,
    forge: Option<&str>,
) -> Option<(&'static Release, InstallationMode)> {
    let release = RELEASES.iter().find(|release| release.game == game)?;
    let mode = match loader {
        ModLoader::Vanilla => InstallationMode::Vanilla,
        ModLoader::Forge if forge == Some(release.forge) => InstallationMode::Forge,
        _ => return None,
    };
    Some((release, mode))
}

fn instance_mode(
    metadata: &InstanceMetadata,
) -> Option<(&'static Release, InstallationMode)> {
    let content = &metadata.applied_content_set;
    mode(
        &content.game_version,
        content.loader,
        content.loader_version.as_deref(),
    )
}

fn instance_available(metadata: &InstanceMetadata) -> bool {
    !metadata.quarantined
        && metadata.instance.install_stage == InstanceInstallStage::Installed
        && matches!(metadata.link, InstanceLink::Unmanaged)
}

pub async fn optifine_status(instance_id: &str) -> Result<OptiFineState> {
    let state = State::get().await?;
    let metadata = crate::state::get_instance(instance_id, &state.pool)
        .await?
        .context("unknown instance")?;
    if !instance_available(&metadata) {
        return Ok(OptiFineState {
            status: OptiFineStatus::Unavailable,
            release: None,
        });
    }
    let Some((release, mode)) = instance_mode(&metadata) else {
        return Ok(OptiFineState {
            status: OptiFineStatus::Unsupported,
            release: None,
        });
    };
    Ok(OptiFineState {
        status: status(&metadata, &state, release, mode).await?,
        release: Some(release.label),
    })
}

async fn status(
    metadata: &InstanceMetadata,
    state: &State,
    release: &Release,
    mode: InstallationMode,
) -> Result<OptiFineStatus> {
    match mode {
        InstallationMode::Vanilla => {
            let directory =
                install_directory(&state.directories, &metadata.instance.id, release);
            match fs::symlink_metadata(&directory).await {
                Ok(info) if info.is_dir() && !super::archive_io::path_is_link(&info) => {
                    let files = launch_jars(&directory);
                    if files.iter().all(|file| regular_file(file)) {
                        Ok(OptiFineStatus::Installed)
                    } else {
                        Ok(OptiFineStatus::RepairNeeded)
                    }
                }
                Ok(_) => Ok(OptiFineStatus::RepairNeeded),
                Err(error) if error.kind() == IoErrorKind::NotFound => {
                    Ok(OptiFineStatus::Available)
                }
                Err(error) => Err(error.into()),
            }
        }
        InstallationMode::Forge => forge_status(metadata, state, release).await,
    }
}

async fn forge_status(
    metadata: &InstanceMetadata,
    state: &State,
    release: &Release,
) -> Result<OptiFineStatus> {
    let destination = state
        .content_store
        .instance_path(
            &metadata.instance.path,
            &format!("mods/{}", release.installer),
        )
        .await?;
    let directory = destination.parent().context("invalid mods directory")?;
    let mut entries = match fs::read_dir(directory).await {
        Ok(entries) => entries,
        Err(error) if error.kind() == IoErrorKind::NotFound => {
            return Ok(OptiFineStatus::Available);
        }
        Err(error) => return Err(error.into()),
    };
    let mut found = OptiFineStatus::Available;
    while let Some(entry) = entries.next_entry().await? {
        let filename = entry.file_name().to_string_lossy().to_ascii_lowercase();
        let jar_name = filename.strip_suffix(".disabled").unwrap_or(&filename);
        if !jar_name.ends_with(".jar")
            || !(jar_name.starts_with("optifine")
                || jar_name.starts_with("preview_optifine"))
        {
            continue;
        }
        if found != OptiFineStatus::Available
            || !jar_name.eq_ignore_ascii_case(release.installer)
            || read_installer(&entry.path(), release).await.is_err()
        {
            return Ok(OptiFineStatus::Conflict);
        }
        found = if filename.ends_with(".disabled") {
            OptiFineStatus::Disabled
        } else {
            OptiFineStatus::Installed
        };
    }
    Ok(found)
}

pub async fn install_optifine(
    instance_id: &str,
    installer: &Path,
) -> Result<OptiFineState> {
    let state = State::get().await?;
    let _activity = state.instance_locks.lock_activity(instance_id).await;
    let metadata = crate::state::get_instance(instance_id, &state.pool)
        .await?
        .context("unknown instance")?;
    ensure!(
        instance_available(&metadata),
        "this instance cannot install OptiFine"
    );
    let (release, installation) = instance_mode(&metadata)
        .context("this Minecraft and loader combination cannot install OptiFine")?;
    let current = status(&metadata, &state, release, installation).await?;
    let installed = OptiFineState {
        status: OptiFineStatus::Installed,
        release: Some(release.label),
    };
    if current == OptiFineStatus::Installed {
        return Ok(installed);
    }
    ensure!(
        current == OptiFineStatus::Available,
        "an existing OptiFine installation must be resolved first"
    );
    ensure!(
        !crate::state::instance_has_running_process(instance_id, &state).await?,
        "stop Minecraft before installing OptiFine"
    );

    let installer = read_installer(installer, release).await?;
    match installation {
        InstallationMode::Vanilla => {
            install_vanilla(instance_id, &installer, &state, release).await?
        }
        InstallationMode::Forge => {
            install_forge(instance_id, &installer, &state, release).await?
        }
    }
    if let Err(error) =
        emit_instance(instance_id, InstancePayloadType::Edited).await
    {
        tracing::warn!(%instance_id, "Could not report completed OptiFine installation: {error}");
    }
    Ok(installed)
}

async fn read_installer(path: &Path, release: &Release) -> Result<Vec<u8>> {
    let file = fs::File::open(path).await?;
    let metadata = file.metadata().await?;
    ensure!(metadata.is_file(), "invalid OptiFine installer file");
    let expected_installer = || {
        ErrorKind::InputError(format!(
            "select the official OptiFine {} installer",
            release.label
        ))
    };
    ensure!(metadata.len() == release.size, expected_installer());
    let mut bytes = Vec::with_capacity(release.size as usize);
    file.take(release.size + 1).read_to_end(&mut bytes).await?;
    ensure!(installer_matches(&bytes, release), expected_installer());
    Ok(bytes)
}

fn installer_matches(bytes: &[u8], release: &Release) -> bool {
    bytes.len() as u64 == release.size
        && format!("{:x}", Sha256::digest(bytes)) == release.sha256
}

async fn install_forge(
    instance_id: &str,
    installer: &[u8],
    state: &State,
    release: &Release,
) -> Result<()> {
    let (mut file, temporary) = state.content_store.temporary().await?;
    file.write_all(installer).await?;
    file.sync_all().await?;
    drop(file);
    let stored = state.content_store.import_file(&temporary, state).await?;
    install_stored_file_if_absent(
        instance_id,
        InstallContent {
            requested_path: &format!("mods/{}", release.installer),
            stored_file: &stored,
            project_type: ProjectType::Mod,
            source_kind: ContentSourceKind::Local,
            origin: None,
            enabled_override: Some(true),
            previous_path: None,
        },
        state,
    )
    .await
}

async fn install_vanilla(
    instance_id: &str,
    installer: &[u8],
    state: &State,
    release: &Release,
) -> Result<()> {
    let context = crate::state::instances::commands::get_instance_launch_context(
        instance_id,
        &state.pool,
    )
    .await?
    .context("unknown instance")?;
    let java = crate::launcher::resolve_java_for_launch(&context, state).await?;
    ensure!(
        java.parsed_version == release.java,
        "OptiFine {} requires Java {} in this launcher",
        release.label,
        release.java
    );
    let _runtime_lease = state.content_store.runtime_cache_lock.read().await;

    let destination = install_directory(&state.directories, instance_id, release);
    let root =
        destination.parent().context("invalid OptiFine directory")?;
    fs::create_dir_all(root).await?;
    let staging = tempfile::tempdir_in(dunce::canonicalize(root)?)?;
    let payload = staging.path().join("installed");
    fs::create_dir(&payload).await?;
    let installer_jar = staging.path().join(release.installer);
    write_file(&installer_jar, installer).await?;
    let client_jar = staging.path().join("minecraft.jar");
    let cached_client = state
        .directories
        .version_dir(release.game)
        .join(format!("{}.jar", release.game));
    fs::copy(&cached_client, &client_jar).await.context(
        "repair the Minecraft installation before installing OptiFine",
    )?;
    ensure!(
        sha1_file_async(&client_jar).await?.1 == release.client_sha1,
        "repair the Minecraft client JAR before installing OptiFine"
    );

    let [patched, wrapper] = launch_jars(&payload);
    write_file(&wrapper, &read_wrapper(installer)?).await?;
    run_patcher(
        &java.path,
        staging.path(),
        &client_jar,
        &installer_jar,
        &patched,
    )
    .await?;
    let patched_file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&patched)
        .context("OptiFine patcher produced no JAR")?;
    let mut archive =
        ZipArchive::new(&patched_file).context("invalid patched OptiFine JAR")?;
    archive
        .by_name(TWEAKER_CLASS)
        .context("patched OptiFine JAR has no tweaker")?;
    drop(archive);
    patched_file.sync_all()?;
    drop(patched_file);
    fs::rename(payload, destination).await?;
    Ok(())
}

fn read_wrapper(installer: &[u8]) -> Result<Vec<u8>> {
    let mut archive = ZipArchive::new(Cursor::new(installer))?;
    let mut file = archive.by_name(WRAPPER_JAR)?;
    let mut bytes = Vec::new();
    Read::by_ref(&mut file)
        .take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= 1024 * 1024, "OptiFine launch wrapper is too large");
    ZipArchive::new(Cursor::new(&bytes))?
        .by_name("net/minecraft/launchwrapper/Launch.class")
        .context("invalid OptiFine launch wrapper")?;
    Ok(bytes)
}

async fn run_patcher(
    java: &str,
    staging: &Path,
    client: &Path,
    installer: &Path,
    output: &Path,
) -> Result<()> {
    let mut command = Command::new(java);
    command
        .current_dir(staging)
        .arg(format!("-Duser.home={}", staging.display()))
        .args(["-Xmx512m", "-cp"])
        .arg(installer)
        .arg("optifine.Patcher")
        .arg(client)
        .arg(installer)
        .arg(output)
        .env_remove("_JAVA_OPTIONS")
        .env_remove("JAVA_TOOL_OPTIONS")
        .env_remove("JDK_JAVA_OPTIONS")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let status = tokio::time::timeout(Duration::from_secs(120), command.status())
        .await
        .context("OptiFine patcher timed out")??;
    ensure!(status.success(), "OptiFine patcher failed ({status})");
    Ok(())
}

async fn write_file(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut file = fs::File::create_new(path).await?;
    file.write_all(bytes).await?;
    file.sync_all().await?;
    Ok(())
}

fn install_directory(
    directories: &DirectoryInfo,
    instance_id: &str,
    release: &Release,
) -> PathBuf {
    directories
        .metadata_dir()
        .join("optifine")
        .join(release.label)
        .join(format!("{:x}", Sha256::digest(instance_id.as_bytes())))
}

fn launch_jars(directory: &Path) -> [PathBuf; 2] {
    [directory.join(PATCHED_JAR), directory.join(WRAPPER_JAR)]
}

fn regular_file(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|info| {
        info.is_file() && !super::archive_io::path_is_link(&info)
    })
}

pub(crate) fn vanilla_launch_jars(
    directories: &DirectoryInfo,
    instance_id: &str,
    game: &str,
    loader: ModLoader,
    java: u32,
) -> Result<Option<[PathBuf; 2]>> {
    let Some((release, InstallationMode::Vanilla)) = mode(game, loader, None) else {
        return Ok(None);
    };
    let directory = install_directory(directories, instance_id, release);
    match std::fs::symlink_metadata(&directory) {
        Ok(info) if info.is_dir() && !super::archive_io::path_is_link(&info) => {}
        Ok(_) => bail!("the OptiFine installation is damaged"),
        Err(error) if error.kind() == IoErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    }
    let files = launch_jars(&directory);
    ensure!(
        files.iter().all(|file| regular_file(file)),
        "the OptiFine installation is damaged"
    );
    ensure!(
        java == release.java,
        "this OptiFine installation requires Java {}",
        release.java
    );
    Ok(Some(files))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chooses_only_supported_minecraft_and_loader_combinations() {
        assert_eq!(
            mode("1.20.1", ModLoader::Vanilla, None)
                .map(|(release, mode)| (release.java, mode)),
            Some((17, InstallationMode::Vanilla))
        );
        assert_eq!(
            mode("1.20.1", ModLoader::Forge, Some("47.2.18")).map(|(_, mode)| mode),
            Some(InstallationMode::Forge),
        );
        assert_eq!(
            mode("1.21.1", ModLoader::Forge, Some("52.0.16"))
                .map(|(release, mode)| (release.java, mode)),
            Some((21, InstallationMode::Forge)),
        );
        assert!(mode("1.20.1", ModLoader::Forge, Some("47.2.0")).is_none());
        assert!(mode("1.20.1", ModLoader::Fabric, None).is_none());
        assert!(mode("1.21.2", ModLoader::Vanilla, None).is_none());
        assert!(!installer_matches(b"not the official installer", &RELEASES[0]));
    }

    #[test]
    fn launch_overlay_belongs_to_one_vanilla_instance_and_java_version() {
        let root = tempfile::tempdir().unwrap();
        let directories = DirectoryInfo {
            settings_dir: root.path().to_owned(),
            config_dir: root.path().to_owned(),
            app_identifier: "optifine-test".to_owned(),
        };
        let installed = install_directory(&directories, "instance", &RELEASES[0]);
        std::fs::create_dir_all(&installed).unwrap();
        for jar in launch_jars(&installed) {
            drop(std::fs::File::create_new(jar).unwrap());
        }

        assert_eq!(
            vanilla_launch_jars(&directories, "instance", "1.20.1", ModLoader::Vanilla, 17)
                .unwrap(),
            Some(launch_jars(&installed))
        );
        assert!(
            vanilla_launch_jars(&directories, "other", "1.20.1", ModLoader::Vanilla, 17)
                .unwrap()
                .is_none()
        );
        assert!(
            vanilla_launch_jars(&directories, "instance", "1.21.1", ModLoader::Vanilla, 21)
                .unwrap()
                .is_none()
        );
        assert!(
            vanilla_launch_jars(&directories, "instance", "1.20.1", ModLoader::Forge, 17)
                .unwrap()
                .is_none()
        );
        assert!(
            vanilla_launch_jars(&directories, "instance", "1.20.1", ModLoader::Vanilla, 21)
                .is_err()
        );
    }
}
