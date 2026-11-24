import { invoke } from '@tauri-apps/api/core'

import type { InstallJobSnapshot } from './install'
import type { InstanceLoader } from './types'

export type DirectoryLauncher = 'pcl2' | 'pclCe' | 'hmcl'
export type FolderLayout = 'shared' | 'isolated'
export type CurseForgeCategory = 'modpack' | 'mod' | 'resourcepack' | 'shader' | 'datapack'

export interface ImportSpec {
	name: string
	gameVersion: string
	loader: InstanceLoader
	loaderVersion: string | null
}

export interface LauncherVersionPreview {
	id: string
	spec: ImportSpec | null
	error: string | null
	suggestedLayout: FolderLayout | null
}

export interface LauncherContentPreview {
	sourcePath: string
	saves: number
	mods: number
	configFiles: number
	missingPaths: string[]
}

export interface ManualFiles {
	directory: string
	metadata: string | null
}

export type DependencyStatus =
	| 'download'
	| 'local'
	| 'manualRequired'
	| 'metadataUnavailable'
	| 'unsupported'

interface DependencyPreview {
	projectId: number
	fileId: number
	required: boolean
	fileName: string | null
	websiteUrl: string | null
	status: DependencyStatus
}

interface CurseForgePreview {
	spec: ImportSpec
	overrideFiles: number
	dependencies: DependencyPreview[]
}

export interface CurseForgeProject {
	id: number
	name: string
	summary: string
	websiteUrl: string | null
}

export interface CurseForgeFile {
	id: number
	name: string
	gameVersions: string[]
	autoDownload: boolean
}

interface CurseForgeContentPreview {
	targets: {
		id: string
		name: string
		gameVersion: string
		loader: InstanceLoader
		satisfiedProjects: number[]
	}[]
	requiredProjects: number[]
	conflictingProjects: number[]
}

type ExternalImportSource =
	| {
			type: 'launcherDirectory'
			path: string
			launcher: DirectoryLauncher
			versionId: string
			layout: FolderLayout
	  }
	| { type: 'curseForgeArchive'; path: string; manual: ManualFiles | null }
	| { type: 'curseForgeOnline'; projectId: number; fileId: number; manual: ManualFiles | null }

export function inspectLauncherVersions(path: string, launcher: DirectoryLauncher) {
	return invoke<LauncherVersionPreview[]>('plugin:import|inspect_launcher_versions', {
		path,
		launcher,
	})
}

export function inspectLauncherContent(path: string, versionId: string, layout: FolderLayout) {
	return invoke<LauncherContentPreview>('plugin:import|inspect_launcher_content', {
		path,
		versionId,
		layout,
	})
}

export function curseforgeApiAvailable() {
	return invoke<boolean>('plugin:import|curseforge_api_available')
}

export function inspectCurseforgeArchive(path: string, manual: ManualFiles | null) {
	return invoke<CurseForgePreview>('plugin:import|curseforge_inspect_archive', { path, manual })
}

export function searchCurseforgeProjects(
	category: CurseForgeCategory,
	query: string,
	page: number,
) {
	return invoke<{ projects: CurseForgeProject[]; hasNext: boolean }>(
		'plugin:import|curseforge_search_modpacks',
		{ category, query, page },
	)
}

export function listCurseforgeFiles(category: CurseForgeCategory, projectId: number, page: number) {
	return invoke<{ files: CurseForgeFile[]; hasNext: boolean }>(
		'plugin:import|curseforge_modpack_files',
		{ category, projectId, page },
	)
}

export function previewCurseforgeContent(
	category: CurseForgeCategory,
	projectId: number,
	fileId: number,
) {
	return invoke<CurseForgeContentPreview>('plugin:import|curseforge_content_preview', {
		category,
		projectId,
		fileId,
	})
}

export function installCurseforgeContent(
	category: CurseForgeCategory,
	projectId: number,
	fileId: number,
	instanceId: string,
	world?: string,
) {
	return invoke<string>('plugin:import|curseforge_install_content', {
		category,
		projectId,
		fileId,
		instanceId,
		world,
	})
}

export function importExternalInstance(source: ExternalImportSource, title: string) {
	return invoke<InstallJobSnapshot>('plugin:import|import_external_instance', { source, title })
}
