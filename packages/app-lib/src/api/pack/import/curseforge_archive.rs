use std::{
    collections::{HashMap, HashSet},
    fs::{self, File},
    io::{self, Read},
    path::{Path, PathBuf},
    sync::LazyLock,
    time::Duration,
};

use anyhow::{Context, bail, ensure};
use reqwest::{Client, Method, Url, header::HeaderValue, redirect::Policy};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use tokio::io::AsyncWriteExt;
use zip::ZipArchive;

use crate::{
    api::instance::archive_io::archive_path,
    install::{InstallPhaseDetails, InstallPhaseId, InstallProgress, InstallProgressReporter},
    state::ModLoader,
    util::fetch::sha1_file_async,
};

use super::{
    external::InstanceSpec,
    launcher_directory::{checked_metadata, metadata_if_present},
};

const API_ROOT: &str = "https://api.curseforge.com/v1";
const API_KEY: &str = "ACL_CURSEFORGE_API_KEY";
pub(super) const MINECRAFT_ID: u32 = 432;
const MODPACK_CLASS: u32 = 4471;
const PAGE_SIZE: u32 = 25;
const MAX_JSON_BYTES: u64 = 16 * 1024 * 1024;
const MAX_ARCHIVE_BYTES: u64 = 16 * 1024 * 1024 * 1024;
const MAX_DEPENDENCIES: usize = 10_000;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CurseForgeCategory {
    #[default]
    Modpack,
    Mod,
    ResourcePack,
    Shader,
    DataPack,
}

impl CurseForgeCategory {
    pub(super) fn class_id(self) -> u32 {
        match self {
            Self::Modpack => MODPACK_CLASS,
            Self::Mod => 6,
            Self::ResourcePack => 12,
            Self::Shader => 6552,
            Self::DataPack => 6945,
        }
    }
}

static API_CLIENT: LazyLock<Client> = LazyLock::new(|| {
    Client::builder().https_only(true).redirect(Policy::none())
        .connect_timeout(Duration::from_secs(15)).timeout(Duration::from_secs(45))
        .build().expect("CurseForge API client")
});

static DOWNLOAD_CLIENT: LazyLock<Client> = LazyLock::new(|| {
    Client::builder().https_only(true).redirect(Policy::custom(|attempt| {
        if attempt.previous().len() >= 5 || !is_content_url(attempt.url()) {
            attempt.error("unsupported CurseForge file redirect")
        } else {
            attempt.follow()
        }
    })).connect_timeout(Duration::from_secs(25)).read_timeout(Duration::from_secs(60))
        .build().expect("CurseForge download client")
});

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManualFiles {
    pub directory: PathBuf,
    pub metadata: Option<PathBuf>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LocalFileMetadata {
    #[serde(rename = "projectID")]
    project_id: u32,
    #[serde(rename = "fileID")]
    file_id: u32,
    class_id: u32,
    file_name: String,
    file_length: u64,
    sha1: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Manifest {
    manifest_type: String,
    manifest_version: u32,
    name: String,
    minecraft: MinecraftSpec,
    files: Vec<ManifestFile>,
    overrides: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MinecraftSpec {
    version: String,
    mod_loaders: Vec<ManifestLoader>,
}

#[derive(Deserialize)]
struct ManifestLoader {
    id: String,
    #[serde(default)]
    primary: bool,
}

#[derive(Clone, Copy, Deserialize, Serialize)]
struct ManifestFile {
    #[serde(rename(deserialize = "projectID", serialize = "projectId"))]
    project_id: u32,
    #[serde(rename(deserialize = "fileID", serialize = "fileId"))]
    file_id: u32,
    required: bool,
}

#[derive(Deserialize)]
struct ApiResponse<T> {
    data: T,
    pagination: Option<Pagination>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Pagination {
    total_count: u32,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ApiProject {
    id: u32,
    game_id: u32,
    class_id: Option<u32>,
    allow_mod_distribution: Option<bool>,
    name: String,
    summary: String,
    links: ApiProjectLinks,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ApiProjectLinks {
    website_url: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ApiFile {
    id: u32,
    game_id: u32,
    mod_id: u32,
    display_name: String,
    pub(super) file_name: String,
    pub(super) file_length: u64,
    download_url: Option<String>,
    is_available: bool,
    is_server_pack: Option<bool>,
    #[serde(default)]
    hashes: Vec<ApiHash>,
    #[serde(default)]
    pub(super) game_versions: Vec<String>,
    dependencies: Vec<ApiFileDependency>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ApiFileDependency {
    pub(super) mod_id: u32,
    pub(super) relation_type: u32,
}

impl ApiFile {
    pub(super) fn dependencies(&self) -> &[ApiFileDependency] {
        &self.dependencies
    }
}

#[derive(Deserialize)]
struct ApiHash {
    algo: u32,
    value: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchPage {
    projects: Vec<SearchProject>,
    has_next: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SearchProject {
    id: u32,
    name: String,
    summary: String,
    website_url: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FilePage {
    files: Vec<SearchFile>,
    has_next: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SearchFile {
    id: u32,
    name: String,
    game_versions: Vec<String>,
    auto_download: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CurseForgePreview {
    pub spec: InstanceSpec,
    override_files: usize,
    pub dependencies: Vec<DependencyPreview>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DependencyPreview {
    #[serde(flatten)]
    manifest: ManifestFile,
    file_name: Option<String>,
    website_url: Option<String>,
    status: DependencyStatus,
}

#[derive(Serialize, Copy, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
enum DependencyStatus {
    Download,
    Local,
    ManualRequired,
    MetadataUnavailable,
    Unsupported,
}

struct Dependency {
    preview: DependencyPreview,
    file: Option<ResolvedFile>,
}

struct ResolvedFile {
    filename: String,
    length: u64,
    sha1: String,
    folder: &'static str,
    url: Option<Url>,
}

fn api_key() -> Option<HeaderValue> {
    let value = std::env::var(API_KEY).ok()?;
    if value.trim().is_empty() { return None; }
    let mut header = HeaderValue::from_str(value.trim()).ok()?;
    header.set_sensitive(true);
    Some(header)
}

pub fn api_available() -> bool { api_key().is_some() }

async fn call_api<T: DeserializeOwned>(method: Method, path: &str, query: &[(&str, String)], body: Option<serde_json::Value>) -> anyhow::Result<ApiResponse<T>> {
    let key = api_key().context("CurseForge requires an approved API key in ACL_CURSEFORGE_API_KEY.")?;
    let mut request = API_CLIENT.request(method, format!("{API_ROOT}{path}"))
        .header("x-api-key", key).header("accept", "application/json").query(query);
    if let Some(body) = body { request = request.json(&body); }
    let mut response = crate::install::control::download_step(request.send()).await??;
    match response.status().as_u16() {
        200 => {}
        401 | 403 => bail!("CurseForge rejected the API key or the requested API access."),
        429 => bail!("The CurseForge API has reached its request limit. Try again later."),
        status => bail!("The CurseForge API returned HTTP {status}."),
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = crate::install::control::download_step(response.chunk()).await?? {
        ensure!((bytes.len() + chunk.len()) as u64 <= MAX_JSON_BYTES, "CurseForge API response exceeds the size limit.");
        bytes.extend_from_slice(&chunk);
    }
    Ok(serde_json::from_slice(&bytes).context("Invalid CurseForge API response.")?)
}

fn page_offset(page: u32) -> u32 { page.min(399) * PAGE_SIZE }

pub async fn search_projects(
    category: CurseForgeCategory,
    query: String,
    page: u32,
) -> crate::Result<SearchPage> {
    let offset = page_offset(page);
    let class_id = category.class_id();
    let response: ApiResponse<Vec<ApiProject>> = call_api(Method::GET, "/mods/search", &[
        ("gameId", MINECRAFT_ID.to_string()), ("classId", class_id.to_string()),
        ("searchFilter", query.trim().chars().take(150).collect()),
        ("index", offset.to_string()), ("pageSize", PAGE_SIZE.to_string()),
    ], None).await?;
    Ok(SearchPage {
        projects: response.data.into_iter().filter(|project| project.game_id == MINECRAFT_ID && project.class_id == Some(class_id))
            .map(|project| SearchProject {
                id: project.id,
                website_url: project_website(&project),
                name: project.name,
                summary: project.summary,
            }).collect(),
        has_next: response.pagination.is_some_and(|p| offset + PAGE_SIZE < p.total_count.min(10_000)),
    })
}

pub async fn project_files(
    category: CurseForgeCategory,
    project_id: u32,
    page: u32,
) -> crate::Result<FilePage> {
    let project = get_project(category, project_id).await?;
    let offset = page_offset(page);
    let response: ApiResponse<Vec<ApiFile>> = call_api(Method::GET, &format!("/mods/{project_id}/files"), &[
        ("index", offset.to_string()), ("pageSize", PAGE_SIZE.to_string()),
    ], None).await?;
    Ok(FilePage {
        files: response.data.into_iter().filter(|file| file.mod_id == project_id && file.game_id == MINECRAFT_ID)
            .map(|file| SearchFile {
                id: file.id,
                auto_download: api_download_url(&project, &file).is_some()
                    && valid_sha1(&file).is_some()
                    && (1..=MAX_ARCHIVE_BYTES).contains(&file.file_length),
                name: file.display_name, game_versions: file.game_versions,
            }).collect(),
        has_next: response.pagination.is_some_and(|p| offset + PAGE_SIZE < p.total_count.min(10_000)),
    })
}

async fn get_project(category: CurseForgeCategory, project_id: u32) -> anyhow::Result<ApiProject> {
    let project: ApiResponse<ApiProject> = call_api(Method::GET, &format!("/mods/{project_id}"), &[], None).await?;
    ensure!(project.data.id == project_id && project.data.game_id == MINECRAFT_ID && project.data.class_id == Some(category.class_id()), "The selected CurseForge project is in a different Minecraft category.");
    Ok(project.data)
}

pub(super) async fn get_project_file(
    category: CurseForgeCategory,
    project_id: u32,
    file_id: u32,
) -> anyhow::Result<(ApiProject, ApiFile)> {
    let route = format!("/mods/{project_id}/files/{file_id}");
    let (project, file) = tokio::try_join!(
        get_project(category, project_id),
        call_api::<ApiFile>(Method::GET, &route, &[], None),
    )?;
    let file = file.data;
    ensure!(file.id == file_id && file.mod_id == project_id && file.game_id == MINECRAFT_ID, "CurseForge returned a different file.");
    Ok((project, file))
}

pub(super) async fn dependency_files(project_id: u32) -> anyhow::Result<Vec<ApiFile>> {
    let route = format!("/mods/{project_id}/files");
    let response: ApiResponse<Vec<ApiFile>> = call_api(
        Method::GET, &route, &[("pageSize", "50".into())], None,
    ).await?;
    Ok(response.data.into_iter().filter(|file| {
        file.mod_id == project_id && file.game_id == MINECRAFT_ID
            && file.is_available && file.is_server_pack != Some(true)
    }).collect())
}

pub async fn inspect_archive(path: PathBuf, manual: Option<ManualFiles>) -> crate::Result<CurseForgePreview> {
    let (manifest, override_files) = tokio::task::spawn_blocking(move || read_zip(&path, None)).await??;
    let spec = manifest_spec(&manifest)?;
    let dependencies = resolve_dependencies(&manifest.files, manual.as_ref()).await?
        .into_iter().map(|dependency| dependency.preview).collect();
    Ok(CurseForgePreview { spec, override_files, dependencies })
}

pub(super) fn prepare(
    local: Option<PathBuf>, online: Option<(u32, u32)>, manual: Option<ManualFiles>,
    staging: PathBuf,
    reporter: InstallProgressReporter, details: InstallPhaseDetails,
) -> impl std::future::Future<Output = crate::Result<InstanceSpec>> + Send {
    Box::pin(prepare_inner(local, online, manual, staging, reporter, details))
}

async fn prepare_inner(
    local: Option<PathBuf>, online: Option<(u32, u32)>, manual: Option<ManualFiles>,
    staging: PathBuf,
    reporter: InstallProgressReporter, details: InstallPhaseDetails,
) -> crate::Result<InstanceSpec> {
    let archive = match (local, online) {
        (Some(path), None) => path,
        (None, Some((project_id, file_id))) => {
            let (project, file) = get_project_file(CurseForgeCategory::Modpack, project_id, file_id).await?;
            let download_url = api_download_url(&project, &file).context("This modpack cannot be downloaded automatically. Get its ZIP from CurseForge and import it locally.")?;
            let sha1 = valid_sha1(&file).context("CurseForge did not provide a SHA-1 for the modpack ZIP.")?;
            let path = staging.join("modpack.zip");
            download_to(&download_url, &path, file.file_length, sha1).await?;
            path
        }
        _ => bail!("Select one CurseForge archive source."),
    };

    let (manifest, staging) = tokio::task::spawn_blocking(move || {
        let staging = staging.join("payload");
        fs::create_dir(&staging)?;
        let (manifest, _) = read_zip(&archive, Some(&staging))?;
        Ok::<_, anyhow::Error>((manifest, staging))
    }).await??;
    let spec = manifest_spec(&manifest)?;
    let dependencies = resolve_dependencies(&manifest.files, manual.as_ref()).await?;
    let missing: Vec<_> = dependencies.iter().filter(|dep| dep.preview.manifest.required && !matches!(dep.preview.status, DependencyStatus::Local | DependencyStatus::Download))
        .map(|dep| format!("{}/{}", dep.preview.manifest.project_id, dep.preview.manifest.file_id)).collect();
    ensure!(missing.is_empty(), "Cannot verify or obtain required CurseForge files: {}", missing.join(", "));

    let mut outputs = HashSet::new();
    let total = dependencies.len() as u64;
    for (index, dependency) in dependencies.into_iter().enumerate() {
        let status = dependency.preview.status;
        if let Some(file) = dependency.file && matches!(status, DependencyStatus::Local | DependencyStatus::Download) {
            let relative = format!("{}/{}", file.folder, file.filename);
            let path = staging.join(&relative);
            ensure!(outputs.insert(relative.to_uppercase()) && !tokio::fs::try_exists(&path).await?, "Conflicting modpack dependency: {relative}");
            tokio::fs::create_dir_all(staging.join(file.folder)).await?;
            if status == DependencyStatus::Local {
                let source = manual.as_ref().context("Missing manual files directory.")?.directory.join(&file.filename);
                let metadata = checked_metadata(&source)?;
                ensure!(metadata.is_file() && metadata.len() == file.length, "Incorrect manual file size: {}", file.filename);
                tokio::fs::copy(source, &path).await?;
                verify_file(&path, file.length, &file.sha1).await?;
            } else if let Some(url) = file.url {
                download_to(&url, &path, file.length, &file.sha1).await?;
            }
        }
        reporter.update(InstallPhaseId::PreparingInstance, Some(InstallProgress { current: (index + 1) as u64, total, secondary: None }), details.clone()).await?;
    }
    Ok(spec)
}

fn read_zip(path: &Path, staging: Option<&Path>) -> anyhow::Result<(Manifest, usize)> {
    let metadata = checked_metadata(path)?;
    ensure!(metadata.is_file() && metadata.len() <= MAX_ARCHIVE_BYTES, "Unsupported CurseForge archive size.");
    let mut archive = ZipArchive::new(File::open(path)?).context("Cannot read this CurseForge ZIP.")?;
    ensure!(archive.len() <= 100_000, "The CurseForge ZIP contains too many entries.");
    let manifest: Manifest = {
        let entry = archive.by_name("manifest.json").context("The ZIP must contain manifest.json at its root.")?;
        ensure!(entry.size() <= MAX_JSON_BYTES && entry.is_file(), "Invalid CurseForge manifest size.");
        let mut bytes = Vec::new();
        entry.take(MAX_JSON_BYTES + 1).read_to_end(&mut bytes)?;
        ensure!(bytes.len() as u64 <= MAX_JSON_BYTES, "Invalid CurseForge manifest size.");
        serde_json::from_slice(bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(&bytes)).context("Invalid CurseForge manifest.json.")?
    };
    ensure!(manifest.manifest_type == "minecraftModpack" && manifest.manifest_version == 1, "Unsupported CurseForge manifest format.");
    ensure!(manifest.files.len() <= MAX_DEPENDENCIES, "The CurseForge manifest lists too many files.");
    let mut pairs = HashSet::new();
    for file in &manifest.files {
        ensure!(file.project_id > 0 && file.file_id > 0 && pairs.insert((file.project_id, file.file_id)), "Invalid or duplicate dependency in the CurseForge manifest.");
    }
    let overrides = archive_path(&manifest.overrides)?;
    let mut entries = HashMap::<String, (PathBuf, bool, bool)>::new();
    let mut override_files = 0;
    let mut total = 0u64;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index)?;
        let name = std::str::from_utf8(entry.name_raw())
            .context("The CurseForge ZIP has an invalid path encoding.")?;
        let name = name.strip_suffix('/').unwrap_or(name);
        let relative = archive_path(name)?;
        let is_dir = entry.is_dir();
        validate_zip_path(&relative, is_dir, &mut entries)?;
        ensure!(entry.unix_mode().is_none_or(|mode| {
            matches!((is_dir, mode & 0o170000), (_, 0) | (true, 0o040000) | (false, 0o100000))
        }), "Unsupported ZIP entry: {name}");
        ensure!(!is_dir || entry.size() == 0, "ZIP directory contains file data: {name}");
        let Ok(relative) = relative.strip_prefix(&overrides) else { continue; };
        if entry.is_dir() || relative.as_os_str().is_empty() { continue; }
        let root = relative.iter().next().and_then(|part| part.to_str()).unwrap_or_default().to_ascii_lowercase();
        ensure!(!matches!(root.as_str(), "versions" | "libraries" | "assets" | "java" | "launcher_accounts.json" | "launcher_profiles.json" | "launcher_profiles_microsoft_store.json" | "pcl" | "hmclversion.cfg"), "Unsupported launcher file in the CurseForge overrides: {name}");
        override_files += 1;
        let size = entry.size();
        total = total.checked_add(size).context("The CurseForge overrides exceed the size limit.")?;
        ensure!(total <= MAX_ARCHIVE_BYTES, "The CurseForge overrides exceed the size limit.");
        if let Some(staging) = staging {
            let path = staging.join(relative);
            fs::create_dir_all(path.parent().context("Missing CurseForge output directory.")?)?;
            let mut output = File::options().create_new(true).write(true).open(path)?;
            let written = io::copy(&mut (&mut entry).take(size + 1), &mut output)?;
            ensure!(written == size, "A CurseForge override has an incorrect size.");
        }
    }
    Ok((manifest, override_files))
}

fn validate_zip_path(
    path: &Path,
    directory: bool,
    entries: &mut HashMap<String, (PathBuf, bool, bool)>,
) -> anyhow::Result<()> {
    for relative in path.ancestors().take_while(|part| !part.as_os_str().is_empty()) {
        let is_entry = relative == path;
        let is_directory = !is_entry || directory;
        let key = relative.to_string_lossy().to_uppercase();
        let (previous_path, previous_directory, written) = entries.entry(key)
            .or_insert_with(|| (relative.to_owned(), is_directory, false));
        ensure!(previous_path.as_path() == relative && *previous_directory == is_directory && (!is_entry || !*written),
            "Conflicting ZIP path: {}", path.display());
        *written |= is_entry;
    }
    Ok(())
}

fn manifest_spec(manifest: &Manifest) -> anyhow::Result<InstanceSpec> {
    let name = manifest.name.trim();
    let game_version = manifest.minecraft.version.trim();
    ensure!(!name.is_empty() && !game_version.is_empty(), "The CurseForge manifest is missing the pack name or Minecraft version.");
    let loaders = &manifest.minecraft.mod_loaders;
    let primary: Vec<_> = loaders.iter().filter(|loader| loader.primary).collect();
    ensure!(primary.len() <= 1 && (loaders.len() <= 1 || !primary.is_empty()), "The CurseForge manifest does not identify the primary mod loader.");
    let selected = primary.first().copied().or_else(|| loaders.first());
    let (loader, loader_version) = match selected {
        None => (ModLoader::Vanilla, None),
        Some(entry) => {
            let (name, version) = entry.id.split_once('-').context("Missing CurseForge mod loader version.")?;
            let loader = match name {
                "forge" => ModLoader::Forge, "neoforge" => ModLoader::NeoForge,
                "fabric" => ModLoader::Fabric, "quilt" => ModLoader::Quilt,
                _ => bail!("Unsupported CurseForge mod loader: {name}"),
            };
            let version = super::external::normalize_loader_version(loader, game_version, version);
            ensure!(version.chars().any(|character| character.is_ascii_digit()), "Missing exact CurseForge mod loader version.");
            (loader, Some(version))
        }
    };
    Ok(InstanceSpec { name: name.to_owned(), game_version: game_version.to_owned(), loader, loader_version })
}

async fn resolve_dependencies(files: &[ManifestFile], manual: Option<&ManualFiles>) -> anyhow::Result<Vec<Dependency>> {
    if files.is_empty() { return Ok(Vec::new()); }
    let mut projects = HashMap::new();
    let mut remote_files = HashMap::new();
    let mut offline_files = HashMap::new();
    if let Some(manual) = manual {
        ensure!(checked_metadata(&manual.directory)?.is_dir(), "Select a directory with CurseForge files.");
        if let Some(path) = &manual.metadata {
            let path = path.clone();
            offline_files = tokio::task::spawn_blocking(move || read_manual_metadata(&path)).await??;
        }
    }
    if api_available() && manual.and_then(|manual| manual.metadata.as_ref()).is_none() {
        for batch in files.chunks(50) {
            let mod_ids: Vec<_> = batch.iter().map(|file| file.project_id).collect();
            let file_ids: Vec<_> = batch.iter().map(|file| file.file_id).collect();
            let (projects_response, files_response) = tokio::try_join!(
                call_api::<Vec<ApiProject>>(Method::POST, "/mods", &[], Some(serde_json::json!({"modIds": mod_ids}))),
                call_api::<Vec<ApiFile>>(Method::POST, "/mods/files", &[], Some(serde_json::json!({"fileIds": file_ids}))),
            )?;
            projects.extend(projects_response.data.into_iter().map(|project| (project.id, project)));
            remote_files.extend(files_response.data.into_iter().map(|file| ((file.mod_id, file.id), file)));
        }
    }
    files.iter().map(|manifest| resolve_dependency(*manifest, manual, projects.get(&manifest.project_id), remote_files.get(&(manifest.project_id, manifest.file_id)), offline_files.get(&(manifest.project_id, manifest.file_id)))).collect()
}

fn resolve_dependency(manifest: ManifestFile, manual: Option<&ManualFiles>, project: Option<&ApiProject>, remote: Option<&ApiFile>, offline: Option<&LocalFileMetadata>) -> anyhow::Result<Dependency> {
    let mut preview = DependencyPreview {
        manifest, file_name: None, status: DependencyStatus::MetadataUnavailable,
        website_url: project.and_then(project_website),
    };
    let metadata = match (offline, project, remote) {
        (Some(file), _, _) => Some((Some(file.class_id), file.file_name.as_str(), file.file_length, Some(file.sha1.as_str()), None)),
        (None, Some(project), Some(file)) if project.game_id == MINECRAFT_ID && file.game_id == MINECRAFT_ID => {
            Some((project.class_id, file.file_name.as_str(), file.file_length, valid_sha1(file), api_download_url(project, file)))
        }
        _ => None,
    };
    let Some((class, filename, length, sha1, url)) = metadata else { return Ok(Dependency { preview, file: None }); };
    preview.file_name = Some(filename.to_owned());
    let Some(folder) = class.and_then(class_folder) else {
        preview.status = DependencyStatus::Unsupported;
        return Ok(Dependency { preview, file: None });
    };
    let Some(sha1) = sha1.filter(|value| is_sha1(value)) else { return Ok(Dependency { preview, file: None }); };
    if length == 0 || length > MAX_ARCHIVE_BYTES || !super::launcher_directory::is_single_name(filename) || archive_path(filename).is_err() {
        preview.status = DependencyStatus::Unsupported;
        return Ok(Dependency { preview, file: None });
    }
    let local = manual.map(|manual| metadata_if_present(&manual.directory.join(filename)))
        .transpose()?.flatten();
    preview.status = match local {
        Some(metadata) if metadata.is_file() && metadata.len() == length => DependencyStatus::Local,
        Some(_) => DependencyStatus::Unsupported,
        None if url.is_some() => DependencyStatus::Download,
        None => DependencyStatus::ManualRequired,
    };
    Ok(Dependency { preview, file: Some(ResolvedFile { filename: filename.to_owned(), length, sha1: sha1.to_ascii_lowercase(), folder, url }) })
}

fn read_manual_metadata(path: &Path) -> anyhow::Result<HashMap<(u32, u32), LocalFileMetadata>> {
    ensure!(checked_metadata(path)?.is_file(), "Select a local CurseForge metadata JSON file.");
    let mut bytes = Vec::new();
    File::open(path)?.take(MAX_JSON_BYTES + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() as u64 <= MAX_JSON_BYTES, "The local CurseForge metadata exceeds the size limit.");
    let entries: Vec<LocalFileMetadata> = serde_json::from_slice(bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(&bytes)).context("Invalid local CurseForge metadata JSON.")?;
    let mut result = HashMap::new();
    for entry in entries {
        ensure!(result.insert((entry.project_id, entry.file_id), entry).is_none(), "Duplicate local CurseForge file metadata.");
    }
    Ok(result)
}

fn class_folder(class: u32) -> Option<&'static str> {
    match class {
        6 => Some("mods"), 12 => Some("resourcepacks"), 6552 => Some("shaderpacks"), 6945 => Some("datapacks"),
        _ => None,
    }
}

pub(super) fn valid_sha1(file: &ApiFile) -> Option<&str> {
    file.hashes.iter().find(|hash| hash.algo == 1 && is_sha1(&hash.value)).map(|hash| hash.value.as_str())
}

fn is_sha1(value: &str) -> bool { value.len() == 40 && value.bytes().all(|byte| byte.is_ascii_hexdigit()) }

fn project_website(project: &ApiProject) -> Option<String> {
    let url = Url::parse(project.links.website_url.as_deref()?).ok()?;
    (url.scheme() == "https" && url.host_str() == Some("www.curseforge.com")
        && url.username().is_empty() && url.password().is_none() && url.port().is_none())
        .then(|| url.to_string())
}

pub(super) fn api_download_url(project: &ApiProject, file: &ApiFile) -> Option<Url> {
    if project.allow_mod_distribution != Some(true) || !file.is_available
        || file.is_server_pack == Some(true)
    {
        return None;
    }
    file.download_url.as_deref().and_then(|url| Url::parse(url).ok()).filter(is_content_url)
}

fn is_content_url(url: &Url) -> bool {
    url.scheme() == "https" && url.username().is_empty() && url.password().is_none() && url.port().is_none()
        && url.host_str().is_some_and(|host| host == "forgecdn.net" || host.ends_with(".forgecdn.net"))
}

pub(super) async fn download_to(url: &Url, target: &Path, length: u64, sha1: &str) -> anyhow::Result<()> {
    ensure!(is_content_url(url) && length > 0 && length <= MAX_ARCHIVE_BYTES, "Invalid CurseForge file download metadata.");
    let key = api_key().context("CurseForge CDN downloads require an approved API key in ACL_CURSEFORGE_API_KEY.")?;
    let request = DOWNLOAD_CLIENT.get(url.clone()).header("x-api-key", key);
    let response = crate::install::control::download_step(request.send()).await?
        .map_err(reqwest::Error::without_url)?;
    if matches!(response.status().as_u16(), 401 | 403) {
        bail!("CurseForge rejected the CDN download. Check API access or get the file on CurseForge and import it locally.");
    }
    let mut response = response.error_for_status().map_err(reqwest::Error::without_url)?;
    let mut output = tokio::fs::OpenOptions::new().write(true).create_new(true).open(target).await?;
    let mut size = 0u64;
    let mut hash = sha1_smol::Sha1::new();
    while let Some(chunk) = crate::install::control::download_step(response.chunk()).await?? {
        size += chunk.len() as u64;
        ensure!(size <= length, "The CurseForge file exceeded the expected size.");
        hash.update(&chunk);
        output.write_all(&chunk).await?;
    }
    output.flush().await?;
    ensure!(size == length && hash.hexdigest().eq_ignore_ascii_case(sha1), "The CurseForge file does not match the expected size and SHA-1.");
    Ok(())
}

async fn verify_file(path: &Path, length: u64, sha1: &str) -> anyhow::Result<()> {
    let (actual_length, actual_hash) = sha1_file_async(path).await?;
    ensure!(actual_length == length && actual_hash.eq_ignore_ascii_case(sha1), "The file {} does not match the expected size and SHA-1.", path.file_name().unwrap_or_default().to_string_lossy());
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    #[test]
    fn api_file_permissions_and_manual_dependencies() -> anyhow::Result<()> {
        let mut project: ApiProject = serde_json::from_str(r#"{"id":42,"gameId":432,"classId":6,"allowModDistribution":false,"name":"Example","summary":"","links":{"websiteUrl":"https://www.curseforge.com/minecraft/mc-mods/example"}}"#)?;
        let file: ApiFile = serde_json::from_str(r#"{"id":7,"gameId":432,"modId":42,"fileName":"example.jar","displayName":"Example","fileLength":3,"downloadUrl":"https://edge.forgecdn.net/files/demo.jar","isAvailable":true,"hashes":[{"algo":1,"value":"a9993e364706816aba3e25717850c26c9cd0d89d"}],"dependencies":[]}"#)?;
        let manifest = ManifestFile { project_id: 42, file_id: 7, required: true };
        assert!(resolve_dependency(manifest, None, Some(&project), Some(&file), None)?.preview.status == DependencyStatus::ManualRequired);
        let directory = tempfile::tempdir()?;
        fs::write(directory.path().join("example.jar"), b"abc")?;
        let manual = ManualFiles { directory: directory.path().to_path_buf(), metadata: None };
        assert!(resolve_dependency(manifest, Some(&manual), Some(&project), Some(&file), None)?.preview.status == DependencyStatus::Local);
        project.allow_mod_distribution = Some(true);
        assert!(resolve_dependency(manifest, None, Some(&project), Some(&file), None)?.preview.status == DependencyStatus::Download);
        Ok(())
    }

    #[test]
    fn zip_manifest_and_overrides_resolve_to_new_instance() -> anyhow::Result<()> {
        let directory = tempfile::tempdir()?;
        let archive = directory.path().join("sample.zip");
        let mut zip = zip::ZipWriter::new(File::create(&archive)?);
        zip.start_file("manifest.json", zip::write::SimpleFileOptions::default())?;
        write!(zip, "{}", r#"{"manifestType":"minecraftModpack","manifestVersion":1,"name":"Sample","minecraft":{"version":"1.20.1","modLoaders":[{"id":"forge-47.2.0","primary":true}]},"files":[{"projectID":42,"fileID":7,"required":true}],"overrides":"overrides"}"#)?;
        zip.start_file("overrides/config/settings.toml", zip::write::SimpleFileOptions::default())?;
        zip.write_all(b"enabled = true")?;
        zip.finish()?;
        let output = tempfile::tempdir()?;
        let (manifest, files) = read_zip(&archive, Some(output.path()))?;
        assert_eq!(files, 1);
        assert_eq!(manifest_spec(&manifest)?.loader, ModLoader::Forge);
        assert_eq!(fs::read(output.path().join("config/settings.toml"))?, b"enabled = true");
        assert!(archive.exists());
        Ok(())
    }

    #[tokio::test]
    async fn manual_dependencies_need_metadata_and_matching_content() -> anyhow::Result<()> {
        let source = tempfile::tempdir()?;
        let metadata = source.path().join("metadata.json");
        let file = source.path().join("example.jar");
        let entry = ManifestFile { project_id: 42, file_id: 7, required: true };
        assert!(resolve_dependency(entry, None, None, None, None)?.preview.status == DependencyStatus::MetadataUnavailable);
        fs::write(&metadata, r#"[{"projectID":42,"fileID":7,"classId":6,"fileName":"example.jar","fileLength":3,"sha1":"a9993e364706816aba3e25717850c26c9cd0d89d"}]"#)?;
        fs::write(&file, b"abc")?;
        let manual = ManualFiles { directory: source.path().to_owned(), metadata: Some(metadata) };
        let actual = resolve_dependencies(&[entry], Some(&manual)).await?;
        assert!(actual[0].preview.status == DependencyStatus::Local);
        let spec = actual[0].file.as_ref().unwrap();
        verify_file(&file, spec.length, &spec.sha1).await?;
        fs::write(&file, b"xyz")?;
        assert!(verify_file(&file, spec.length, &spec.sha1).await.is_err());
        fs::write(&file, b"abcd")?;
        assert!(verify_file(&file, spec.length, &spec.sha1).await.is_err());
        Ok(())
    }
}
