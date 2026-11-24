use std::path::PathBuf;

use crate::api::Result;
use theseus::pack::import::ImportLauncherType;

use theseus::pack::import;
use theseus::pack::import::{curseforge_archive, curseforge_content, external::ExternalImportSource, launcher_directory};

pub fn init<R: tauri::Runtime>() -> tauri::plugin::TauriPlugin<R> {
    tauri::plugin::Builder::new("import")
        .invoke_handler(tauri::generate_handler![
            get_importable_instances,
            is_valid_importable_instance,
            get_default_launcher_path,
            inspect_launcher_versions,
            inspect_launcher_content,
            curseforge_api_available,
            curseforge_inspect_archive,
            curseforge_search_modpacks,
            curseforge_modpack_files,
            curseforge_content_preview,
            curseforge_install_content,
            import_external_instance,
        ])
        .build()
}

#[tauri::command]
pub async fn inspect_launcher_versions(
    path: PathBuf,
    launcher: launcher_directory::LauncherDirectoryType,
) -> Result<Vec<launcher_directory::LauncherVersionPreview>> {
    Ok(launcher_directory::inspect_versions(path, launcher).await?)
}

#[tauri::command]
pub async fn inspect_launcher_content(
    path: PathBuf,
    version_id: String,
    layout: launcher_directory::FolderLayout,
) -> Result<launcher_directory::LauncherContentPreview> {
    Ok(launcher_directory::inspect_content(path, version_id, layout).await?)
}

#[tauri::command]
pub fn curseforge_api_available() -> bool {
    curseforge_archive::api_available()
}

#[tauri::command]
pub async fn curseforge_inspect_archive(
    path: PathBuf,
    manual: Option<curseforge_archive::ManualFiles>,
) -> Result<curseforge_archive::CurseForgePreview> {
    Ok(curseforge_archive::inspect_archive(path, manual).await?)
}

#[tauri::command]
pub async fn curseforge_search_modpacks(
    query: String,
    page: u32,
    category: Option<curseforge_archive::CurseForgeCategory>,
) -> Result<curseforge_archive::SearchPage> {
    Ok(curseforge_archive::search_projects(category.unwrap_or_default(), query, page).await?)
}

#[tauri::command]
pub async fn curseforge_modpack_files(
    project_id: u32,
    page: u32,
    category: Option<curseforge_archive::CurseForgeCategory>,
) -> Result<curseforge_archive::FilePage> {
    Ok(curseforge_archive::project_files(category.unwrap_or_default(), project_id, page).await?)
}

#[tauri::command]
pub async fn curseforge_content_preview(
    category: curseforge_archive::CurseForgeCategory,
    project_id: u32,
    file_id: u32,
) -> Result<curseforge_content::ContentInstallPreview> {
    Ok(curseforge_content::preview_content_install(category, project_id, file_id).await?)
}

#[tauri::command]
pub async fn curseforge_install_content(
    category: curseforge_archive::CurseForgeCategory,
    project_id: u32,
    file_id: u32,
    instance_id: String,
    world: Option<String>,
) -> Result<String> {
    Ok(curseforge_content::install_content(category, project_id, file_id, instance_id, world).await?)
}

#[tauri::command]
pub async fn import_external_instance(
    source: ExternalImportSource,
    title: String,
) -> Result<theseus::install::InstallJobSnapshot> {
    Ok(theseus::install::runner::import_external_instance(source, title).await?)
}

/// Gets a list of importable instances from a launcher type and base path
/// eg: get_importable_instances(ImportLauncherType::MultiMC, PathBuf::from("C:/MultiMC"))
/// returns ["Instance 1", "Instance 2"]
#[tauri::command]
pub async fn get_importable_instances(
    launcher_type: ImportLauncherType,
    base_path: PathBuf,
) -> Result<Vec<String>> {
    Ok(import::get_importable_instances(launcher_type, base_path).await?)
}

/// Checks if this instance is valid for importing, given a certain launcher type
/// eg: is_valid_importable_instance(PathBuf::from("C:/MultiMC/Instance 1"), ImportLauncherType::MultiMC)
#[tauri::command]
pub async fn is_valid_importable_instance(
    instance_folder: PathBuf,
    launcher_type: ImportLauncherType,
) -> Result<bool> {
    Ok(
        import::is_valid_importable_instance(instance_folder, launcher_type)
            .await,
    )
}

/// Returns the default path for the given launcher type
/// None if it can't be found or doesn't exist
#[tauri::command]
pub async fn get_default_launcher_path(
    launcher_type: ImportLauncherType,
) -> Result<Option<PathBuf>> {
    Ok(import::get_default_launcher_path(launcher_type))
}
