<script setup lang="ts">
import { BoxImportIcon, DownloadIcon, FolderSearchIcon } from '@modrinth/assets'
import {
	Button,
	ButtonLink,
	defineMessages,
	formatProjectTypeSentence,
	Input,
	useVIntl,
} from '@modrinth/ui'
import { open } from '@tauri-apps/plugin-dialog'
import { computed, ref, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'

import {
	type CurseForgeCategory,
	type CurseForgeFile,
	type CurseForgeProject,
	curseforgeApiAvailable,
	type DirectoryLauncher,
	type FolderLayout,
	importExternalInstance,
	installCurseforgeContent,
	inspectCurseforgeArchive,
	inspectLauncherContent,
	inspectLauncherVersions,
	listCurseforgeFiles,
	previewCurseforgeContent,
	searchCurseforgeProjects,
} from '@/helpers/external-import'
import { installJobInstanceId, wait_for_install_job } from '@/helpers/install'
import { get_instance_worlds, type SingleplayerWorld } from '@/helpers/worlds'
import { injectAppEvents } from '@/providers/app-events'

const props = defineProps<{ mode: 'launcher' | 'curseforge' }>()
const route = useRoute()
const router = useRouter()
const appEvents = injectAppEvents()
const { formatMessage } = useVIntl()

const messages = defineMessages({
	launcherTitle: {
		id: 'app.external-import.launcher-title',
		defaultMessage: 'Import from PCL2, PCL-CE, or HMCL',
	},
	curseforgeTitle: {
		id: 'app.external-import.curseforge-title',
		defaultMessage: 'CurseForge content',
	},
	launcherDescription: {
		id: 'app.external-import.launcher-description',
		defaultMessage:
			'Select the launcher folder, .minecraft folder, or a version folder. ACL copies the selected game data into a new instance.',
	},
	launcher: { id: 'app.external-import.launcher', defaultMessage: 'Source launcher' },
	chooseDirectory: {
		id: 'app.external-import.choose-directory',
		defaultMessage: 'Choose directory',
	},
	version: { id: 'app.external-import.version', defaultMessage: 'Minecraft version' },
	selectVersion: { id: 'app.external-import.select-version', defaultMessage: 'Select a version' },
	layout: { id: 'app.external-import.layout', defaultMessage: 'Game data directory' },
	selectLayout: {
		id: 'app.external-import.select-layout',
		defaultMessage: 'Select the layout used by your launcher',
	},
	shared: { id: 'app.external-import.shared', defaultMessage: 'Shared Minecraft folder' },
	isolated: { id: 'app.external-import.isolated', defaultMessage: 'Isolated version folder' },
	layoutHint: {
		id: 'app.external-import.layout-hint',
		defaultMessage:
			'Confirm the folder that contains the saves and mods for this version. Some launchers use a global isolation setting.',
	},
	sourceSummary: {
		id: 'app.external-import.source-summary',
		defaultMessage: '{saves} saves · {mods} mod files · {config} config files',
	},
	missing: {
		id: 'app.external-import.missing',
		defaultMessage: 'Missing from this directory: {paths}',
	},
	unsupportedVersion: {
		id: 'app.external-import.unsupported-version',
		defaultMessage: 'Unsupported format',
	},
	versionSummary: {
		id: 'app.external-import.version-summary',
		defaultMessage: 'Minecraft {version} · {loader}',
	},
	archive: { id: 'app.external-import.archive', defaultMessage: 'Import a CurseForge ZIP' },
	chooseArchive: { id: 'app.external-import.choose-archive', defaultMessage: 'Choose ZIP' },
	browse: { id: 'app.external-import.browse', defaultMessage: 'Browse online' },
	apiUnavailable: {
		id: 'app.external-import.api-unavailable',
		defaultMessage:
			'Online browsing requires an approved CurseForge API key in the ACL_CURSEFORGE_API_KEY environment variable. Local ZIP previews are available.',
	},
	search: { id: 'app.external-import.search', defaultMessage: 'Search modpacks' },
	searchContent: { id: 'app.external-import.search-content', defaultMessage: 'Search CurseForge' },
	category: { id: 'app.external-import.category', defaultMessage: 'Content type' },
	noContent: { id: 'app.external-import.no-content', defaultMessage: 'No projects found.' },
	previous: { id: 'app.external-import.previous', defaultMessage: 'Previous' },
	next: { id: 'app.external-import.next', defaultMessage: 'Next' },
	noResults: { id: 'app.external-import.no-results', defaultMessage: 'No modpacks found.' },
	noFiles: { id: 'app.external-import.no-files', defaultMessage: 'No versions found.' },
	chooseVersion: { id: 'app.external-import.choose-version', defaultMessage: 'See versions' },
	manualPack: {
		id: 'app.external-import.manual-pack',
		defaultMessage: 'Get this ZIP on CurseForge and import it locally.',
	},
	manualContent: {
		id: 'app.external-import.manual-content',
		defaultMessage: 'Open this file on CurseForge',
	},
	selectTarget: {
		id: 'app.external-import.select-target',
		defaultMessage: 'Choose a compatible instance',
	},
	noTargets: {
		id: 'app.external-import.no-targets',
		defaultMessage:
			'No compatible installed instances are available. Stop Minecraft if the target instance is running.',
	},
	requiredProjects: {
		id: 'app.external-import.required-projects',
		defaultMessage:
			'Required CurseForge project IDs: {ids}. ACL could not verify these dependencies for this file.',
	},
	requiredProjectList: {
		id: 'app.external-import.required-project-list',
		defaultMessage: 'Required CurseForge projects: {ids}. Choose an instance to check them.',
	},
	satisfiedProjects: {
		id: 'app.external-import.satisfied-projects',
		defaultMessage: 'Verified in this instance: {ids}',
	},
	conflictingProjects: {
		id: 'app.external-import.conflicting-projects',
		defaultMessage: 'ACL could not verify conflicts with CurseForge project IDs: {ids}.',
	},
	chooseWorld: { id: 'app.external-import.choose-world', defaultMessage: 'Choose a local world' },
	noWorld: {
		id: 'app.external-import.no-world',
		defaultMessage: 'No unlocked singleplayer worlds were found in this instance.',
	},
	installContent: { id: 'app.external-import.install-content', defaultMessage: 'Install content' },
	contentInstalled: {
		id: 'app.external-import.content-installed',
		defaultMessage: 'Installed to {path}',
	},
	manualFiles: {
		id: 'app.external-import.manual-files',
		defaultMessage: 'Manually downloaded files',
	},
	manualDescription: {
		id: 'app.external-import.manual-description',
		defaultMessage:
			'Select a folder with the required files you downloaded from CurseForge. ACL checks each file’s exact length and SHA-1 against CurseForge metadata or your local metadata JSON.',
	},
	chooseMetadata: {
		id: 'app.external-import.choose-metadata',
		defaultMessage: 'Choose metadata JSON',
	},
	metadataHint: {
		id: 'app.external-import.metadata-hint',
		defaultMessage: 'Offline metadata format',
	},
	metadataFields: {
		id: 'app.external-import.metadata-fields',
		defaultMessage:
			'Provide a JSON array with projectID, fileID, classId, fileName, fileLength (bytes), and sha1 for each file. A standard CurseForge manifest contains only the file IDs and requires separate verification metadata.',
	},
	clear: { id: 'app.external-import.clear', defaultMessage: 'Clear selection' },
	overrideCount: {
		id: 'app.external-import.override-count',
		defaultMessage: '{count} override files',
	},
	missingRequired: {
		id: 'app.external-import.missing-required',
		defaultMessage:
			'{count} required files are unresolved. Supply those files with verifiable metadata to continue.',
	},
	optional: { id: 'app.external-import.optional', defaultMessage: 'Optional' },
	download: { id: 'app.external-import.download', defaultMessage: 'Automatic download' },
	local: { id: 'app.external-import.local', defaultMessage: 'Local file selected' },
	manualRequired: {
		id: 'app.external-import.manual-required',
		defaultMessage: 'Manual file needed',
	},
	metadataUnavailable: {
		id: 'app.external-import.metadata-unavailable',
		defaultMessage: 'Verification metadata unavailable',
	},
	unsupported: { id: 'app.external-import.unsupported', defaultMessage: 'Unsupported file' },
	working: { id: 'app.external-import.working', defaultMessage: 'Reading source…' },
	installing: {
		id: 'app.external-import.installing',
		defaultMessage: 'Import queued. Follow its progress in Downloads.',
	},
	import: { id: 'app.external-import.import', defaultMessage: 'Import instance' },
	failed: {
		id: 'app.external-import.failed',
		defaultMessage: 'Unable to read or import this source.',
	},
})

const busy = ref(false)
const installing = ref(false)
const errorMessage = ref('')
const launcher = ref<DirectoryLauncher>('pcl2')
const launcherPath = ref('')
const versions = ref<Awaited<ReturnType<typeof inspectLauncherVersions>>>([])
const version = ref('')
const layout = ref<FolderLayout | ''>('')
const gameData = ref<Awaited<ReturnType<typeof inspectLauncherContent>> | null>(null)
const selectedVersion = computed(() => versions.value.find((entry) => entry.id === version.value))

const apiAvailable = ref<boolean | null>(null)
const archivePath = ref('')
const archive = ref<Awaited<ReturnType<typeof inspectCurseforgeArchive>> | null>(null)
const localFiles = ref('')
const metadataPath = ref('')
const manual = computed(() =>
	localFiles.value ? { directory: localFiles.value, metadata: metadataPath.value || null } : null,
)
const missingRequired = computed(
	() =>
		archive.value?.dependencies.filter(
			(file) => file.required && file.status !== 'local' && file.status !== 'download',
		).length ?? 0,
)

const search = ref('')
const activeQuery = ref('')
const categories: CurseForgeCategory[] = ['modpack', 'mod', 'resourcepack', 'shader', 'datapack']
const category = ref<CurseForgeCategory>('modpack')
const searchMessage = computed(() =>
	category.value === 'modpack' ? messages.search : messages.searchContent,
)
const manualDownloadMessage = computed(() =>
	category.value === 'modpack' ? messages.manualPack : messages.manualContent,
)
const results = ref<Awaited<ReturnType<typeof searchCurseforgeProjects>> | null>(null)
const resultPage = ref(0)
const project = ref<CurseForgeProject | null>(null)
const files = ref<Awaited<ReturnType<typeof listCurseforgeFiles>> | null>(null)
const filePage = ref(0)
const selectedFile = ref<CurseForgeFile | null>(null)
const contentPreview = ref<Awaited<ReturnType<typeof previewCurseforgeContent>> | null>(null)
const targetId = ref('')
const world = ref('')
const worlds = ref<SingleplayerWorld[]>([])
const installedPath = ref('')
const selectedTarget = computed(() =>
	contentPreview.value?.targets.find((target) => target.id === targetId.value),
)
const missingProjects = computed(() => {
	const target = selectedTarget.value
	return target
		? (contentPreview.value?.requiredProjects ?? []).filter(
				(id) => !target.satisfiedProjects.includes(id),
			)
		: []
})
const contentBlocked = computed(() =>
	Boolean(missingProjects.value.length || contentPreview.value?.conflictingProjects.length),
)

async function run(action: () => Promise<void>) {
	if (busy.value || installing.value) return
	busy.value = true
	errorMessage.value = ''
	try {
		await action()
	} catch (error) {
		setError(error)
	} finally {
		busy.value = false
	}
}

function setError(error: unknown) {
	errorMessage.value =
		typeof error === 'object' &&
		error !== null &&
		'message' in error &&
		typeof error.message === 'string'
			? error.message
			: formatMessage(messages.failed)
}

async function chooseLauncherDirectory() {
	await run(async () => {
		const selection = await open({ directory: true, multiple: false })
		if (typeof selection !== 'string') return
		launcherPath.value = selection
		await loadLauncherVersions()
	})
}

async function loadLauncherVersions() {
	versions.value = []
	version.value = ''
	layout.value = ''
	gameData.value = null
	if (!launcherPath.value) return
	versions.value = await inspectLauncherVersions(launcherPath.value, launcher.value)
	if (versions.value.length === 1) {
		version.value = versions.value[0].id
		await chooseVersion()
	}
}

async function chooseVersion() {
	layout.value = selectedVersion.value?.suggestedLayout ?? ''
	await loadGameData()
}

async function loadGameData() {
	gameData.value = null
	if (!selectedVersion.value?.spec || !layout.value) return
	gameData.value = await inspectLauncherContent(launcherPath.value, version.value, layout.value)
}

async function chooseArchive() {
	await run(async () => {
		const selection = await open({
			multiple: false,
			filters: [{ name: 'CurseForge', extensions: ['zip'] }],
		})
		if (typeof selection !== 'string') return
		archivePath.value = selection
		await loadArchive()
	})
}

async function loadArchive() {
	archive.value = null
	if (archivePath.value) {
		archive.value = await inspectCurseforgeArchive(archivePath.value, manual.value)
	}
}

async function chooseManualFiles() {
	await run(async () => {
		const selection = await open({ directory: true, multiple: false })
		if (typeof selection !== 'string') return
		localFiles.value = selection
		await loadArchive()
	})
}

async function chooseMetadata() {
	await run(async () => {
		const selection = await open({
			multiple: false,
			filters: [{ name: 'JSON', extensions: ['json'] }],
		})
		if (typeof selection !== 'string') return
		metadataPath.value = selection
		await loadArchive()
	})
}

async function clearManualFiles() {
	await run(async () => {
		localFiles.value = ''
		metadataPath.value = ''
		await loadArchive()
	})
}

async function loadProjects(page: number) {
	await run(async () => {
		if (page === 0) activeQuery.value = search.value.trim()
		project.value = null
		files.value = null
		resetContentPreview()
		results.value = await searchCurseforgeProjects(category.value, activeQuery.value, page)
		resultPage.value = page
	})
}

async function loadFiles(selected: CurseForgeProject, page: number) {
	await run(async () => {
		project.value = selected
		files.value = null
		resetContentPreview()
		files.value = await listCurseforgeFiles(category.value, selected.id, page)
		filePage.value = page
	})
}

function resetContentPreview() {
	selectedFile.value = null
	contentPreview.value = null
	targetId.value = ''
	world.value = ''
	worlds.value = []
	installedPath.value = ''
}

function changeCategory() {
	project.value = null
	files.value = null
	results.value = null
	resetContentPreview()
}

async function loadApplicableWorlds() {
	world.value = ''
	worlds.value = []
	if (category.value !== 'datapack' || !targetId.value) return
	worlds.value = (await get_instance_worlds(targetId.value)).filter(
		(entry): entry is SingleplayerWorld => entry.type === 'singleplayer' && !entry.locked,
	)
}

async function chooseOnlineFile(file: CurseForgeFile) {
	const selected = project.value
	if (!selected || !file.autoDownload) return
	if (category.value === 'modpack') {
		await installOnline(file)
		return
	}
	await run(async () => {
		resetContentPreview()
		contentPreview.value = await previewCurseforgeContent(category.value, selected.id, file.id)
		selectedFile.value = file
		const suggestedTarget = route.query.target
		if (
			typeof suggestedTarget === 'string' &&
			contentPreview.value.targets.some((entry) => entry.id === suggestedTarget)
		) {
			targetId.value = suggestedTarget
			await loadApplicableWorlds()
		}
	})
}

async function installSelectedContent() {
	const selected = project.value
	const file = selectedFile.value
	if (
		!selected ||
		!file ||
		!targetId.value ||
		contentBlocked.value ||
		(category.value === 'datapack' && !world.value)
	)
		return
	await run(async () => {
		installedPath.value = await installCurseforgeContent(
			category.value,
			selected.id,
			file.id,
			targetId.value,
			world.value || undefined,
		)
	})
}

async function runImport(source: Parameters<typeof importExternalInstance>[0], title: string) {
	if (busy.value || installing.value) return
	installing.value = true
	errorMessage.value = ''
	const routeAtStart = route.fullPath
	try {
		const job = await importExternalInstance(source, title)
		const installed = await wait_for_install_job(appEvents, job.job_id)
		const instanceId = installJobInstanceId(installed)
		if (instanceId && router.currentRoute.value.fullPath === routeAtStart) {
			await router.push(`/instance/${encodeURIComponent(instanceId)}`)
		}
	} catch (error) {
		setError(error)
	} finally {
		installing.value = false
	}
}

function importLauncher() {
	if (!selectedVersion.value?.spec || !gameData.value || !layout.value) return
	return runImport(
		{
			type: 'launcherDirectory',
			path: launcherPath.value,
			launcher: launcher.value,
			versionId: version.value,
			layout: layout.value,
		},
		selectedVersion.value.spec.name,
	)
}

function importArchive() {
	if (!archive.value || missingRequired.value) return
	return runImport(
		{ type: 'curseForgeArchive', path: archivePath.value, manual: manual.value },
		archive.value.spec.name,
	)
}

function installOnline(file: CurseForgeFile) {
	if (!project.value || !file.autoDownload) return
	return runImport(
		{
			type: 'curseForgeOnline',
			projectId: project.value.id,
			fileId: file.id,
			manual: manual.value,
		},
		project.value.name,
	)
}

watch(
	() => [props.mode, route.path, route.query.category] as const,
	async ([mode, path]) => {
		if (mode !== 'curseforge' || path !== '/import/curseforge') return
		await run(async () => {
			const requestedCategory =
				categories.find((value) => value === route.query.category) ?? 'modpack'
			if (category.value !== requestedCategory) {
				category.value = requestedCategory
				changeCategory()
			}
			apiAvailable.value = await curseforgeApiAvailable()
			const initialArchive = window.history.state?.archivePath
			if (typeof initialArchive === 'string') {
				archivePath.value = initialArchive
				await loadArchive()
			}
		})
	},
	{ immediate: true },
)
</script>

<template>
	<main class="mx-auto flex w-full max-w-4xl flex-col gap-4 p-6">
		<header>
			<h1 class="m-0 text-2xl font-bold text-contrast">
				{{ formatMessage(mode === 'launcher' ? messages.launcherTitle : messages.curseforgeTitle) }}
			</h1>
			<p v-if="mode === 'launcher'" class="mb-0 text-secondary">
				{{ formatMessage(messages.launcherDescription) }}
			</p>
		</header>
		<p
			v-if="errorMessage"
			class="m-0 break-words rounded-xl bg-surface-3 p-3 text-red"
			role="alert"
		>
			{{ errorMessage }}
		</p>
		<p v-if="busy || installing" class="m-0 text-secondary" role="status">
			{{ formatMessage(installing ? messages.installing : messages.working) }}
		</p>

		<fieldset :disabled="busy || installing" class="m-0 flex min-w-0 flex-col gap-4 border-0 p-0">
			<section v-if="mode === 'launcher'" class="flex flex-col gap-4 rounded-2xl bg-surface-2 p-4">
				<div class="flex flex-wrap items-end gap-3">
					<label class="flex flex-col gap-1 text-sm font-semibold">
						{{ formatMessage(messages.launcher) }}
						<select v-model="launcher" class="picker" @change="run(loadLauncherVersions)">
							<option value="pcl2">PCL2</option>
							<option value="pclCe">PCL-CE</option>
							<option value="hmcl">HMCL</option>
						</select>
					</label>
					<Button @click="chooseLauncherDirectory"
						><FolderSearchIcon />{{ formatMessage(messages.chooseDirectory) }}</Button
					>
				</div>
				<p v-if="launcherPath" class="m-0 break-all text-sm text-secondary">{{ launcherPath }}</p>
				<label v-if="versions.length" class="flex flex-col gap-1 text-sm font-semibold">
					{{ formatMessage(messages.version) }}
					<select v-model="version" class="picker" @change="run(chooseVersion)">
						<option disabled value="">{{ formatMessage(messages.selectVersion) }}</option>
						<option v-for="entry in versions" :key="entry.id" :value="entry.id">
							{{ entry.id
							}}{{ entry.error ? ` (${formatMessage(messages.unsupportedVersion)})` : '' }}
						</option>
					</select>
				</label>
				<p v-if="selectedVersion?.error" class="m-0 text-red">{{ selectedVersion.error }}</p>
				<template v-if="selectedVersion?.spec">
					<p class="m-0 text-secondary">
						{{
							formatMessage(messages.versionSummary, {
								version: selectedVersion.spec.gameVersion,
								loader: [selectedVersion.spec.loader, selectedVersion.spec.loaderVersion]
									.filter(Boolean)
									.join(' '),
							})
						}}
					</p>
					<label class="flex flex-col gap-1 text-sm font-semibold">
						{{ formatMessage(messages.layout) }}
						<select v-model="layout" class="picker" @change="run(loadGameData)">
							<option disabled value="">{{ formatMessage(messages.selectLayout) }}</option>
							<option value="shared">{{ formatMessage(messages.shared) }}</option>
							<option value="isolated">{{ formatMessage(messages.isolated) }}</option>
						</select>
					</label>
					<p class="m-0 text-sm text-secondary">{{ formatMessage(messages.layoutHint) }}</p>
				</template>
				<div v-if="gameData" class="flex flex-col gap-2 rounded-xl bg-surface-3 p-3">
					<strong>{{
						formatMessage(messages.sourceSummary, {
							saves: gameData.saves,
							mods: gameData.mods,
							config: gameData.configFiles,
						})
					}}</strong>
					<span class="break-all text-sm text-secondary">{{ gameData.sourcePath }}</span>
					<span v-if="gameData.missingPaths.length" class="text-sm text-secondary">{{
						formatMessage(messages.missing, { paths: gameData.missingPaths.join(', ') })
					}}</span>
				</div>
				<Button
					v-if="selectedVersion?.spec"
					type="colored"
					color="brand"
					class="w-fit"
					:disabled="!gameData"
					@click="importLauncher"
				>
					<BoxImportIcon />{{ formatMessage(messages.import) }}
				</Button>
			</section>
			<template v-else>
				<section class="flex flex-col gap-3 rounded-2xl bg-surface-2 p-4">
					<h2 class="m-0 text-lg text-contrast">{{ formatMessage(messages.archive) }}</h2>
					<Button class="w-fit" @click="chooseArchive"
						><BoxImportIcon />{{ formatMessage(messages.chooseArchive) }}</Button
					>
					<p v-if="archivePath" class="m-0 break-all text-sm text-secondary">{{ archivePath }}</p>
					<div v-if="archive" class="flex flex-col gap-2">
						<strong>{{ archive.spec.name }}</strong>
						<span class="text-sm text-secondary"
							>{{
								formatMessage(messages.versionSummary, {
									version: archive.spec.gameVersion,
									loader: [archive.spec.loader, archive.spec.loaderVersion]
										.filter(Boolean)
										.join(' '),
								})
							}}
							· {{ formatMessage(messages.overrideCount, { count: archive.overrideFiles }) }}</span
						>
						<p v-if="missingRequired" class="m-0 text-red">
							{{ formatMessage(messages.missingRequired, { count: missingRequired }) }}
						</p>
						<ul
							v-if="archive.dependencies.length"
							class="m-0 max-h-64 list-none divide-y divide-surface-4 overflow-auto rounded-xl bg-surface-3 p-0"
						>
							<li
								v-for="dependency in archive.dependencies"
								:key="`${dependency.projectId}/${dependency.fileId}`"
								class="flex flex-wrap items-center justify-between gap-2 p-2 text-sm"
							>
								<a
									v-if="dependency.websiteUrl"
									:href="dependency.websiteUrl"
									target="_blank"
									rel="noopener noreferrer"
									>{{ dependency.fileName ?? `${dependency.projectId}/${dependency.fileId}` }}</a
								>
								<span v-else class="break-all">{{
									dependency.fileName ?? `${dependency.projectId}/${dependency.fileId}`
								}}</span>
								<span class="text-secondary"
									>{{ formatMessage(messages[dependency.status])
									}}{{ dependency.required ? '' : ` · ${formatMessage(messages.optional)}` }}</span
								>
							</li>
						</ul>
						<Button
							type="colored"
							color="brand"
							class="w-fit"
							:disabled="missingRequired > 0"
							@click="importArchive"
						>
							<BoxImportIcon />{{ formatMessage(messages.import) }}
						</Button>
					</div>
				</section>
				<section class="flex flex-col gap-3 rounded-2xl bg-surface-2 p-4">
					<h2 class="m-0 text-lg text-contrast">{{ formatMessage(messages.manualFiles) }}</h2>
					<p class="m-0 text-sm text-secondary">{{ formatMessage(messages.manualDescription) }}</p>
					<div class="flex flex-wrap gap-2">
						<Button @click="chooseManualFiles"
							><FolderSearchIcon />{{ formatMessage(messages.chooseDirectory) }}</Button
						>
						<Button :disabled="!localFiles" @click="chooseMetadata">{{
							formatMessage(messages.chooseMetadata)
						}}</Button>
						<Button v-if="localFiles" type="quiet" @click="clearManualFiles">{{
							formatMessage(messages.clear)
						}}</Button>
					</div>
					<p v-if="localFiles" class="m-0 break-all text-sm text-secondary">{{ localFiles }}</p>
					<p v-if="metadataPath" class="m-0 break-all text-sm text-secondary">{{ metadataPath }}</p>
					<details v-if="!apiAvailable" class="text-sm text-secondary">
						<summary class="cursor-pointer">{{ formatMessage(messages.metadataHint) }}</summary>
						<p class="mb-0">{{ formatMessage(messages.metadataFields) }}</p>
					</details>
				</section>
				<section class="flex flex-col gap-3 rounded-2xl bg-surface-2 p-4">
					<h2 class="m-0 text-lg text-contrast">{{ formatMessage(messages.browse) }}</h2>
					<p v-if="apiAvailable === false" class="m-0 text-sm text-secondary">
						{{ formatMessage(messages.apiUnavailable) }}
					</p>
					<template v-if="apiAvailable">
						<form class="flex flex-wrap items-end gap-2" @submit.prevent="loadProjects(0)">
							<label class="flex flex-col gap-1 text-sm font-semibold">
								{{ formatMessage(messages.category) }}
								<select v-model="category" class="picker" @change="changeCategory">
									<option v-for="option in categories" :key="option" :value="option">
										{{ formatProjectTypeSentence(formatMessage, option, 2) }}
									</option>
								</select>
							</label>
							<Input
								v-model="search"
								:aria-label="formatMessage(searchMessage)"
								:placeholder="formatMessage(searchMessage)"
								class="flex-1"
							/>
							<Button native-type="submit">{{ formatMessage(searchMessage) }}</Button>
						</form>
						<template v-if="selectedFile && contentPreview">
							<div class="flex flex-wrap items-center gap-3">
								<Button size="sm" @click="resetContentPreview">{{
									formatMessage(messages.chooseVersion)
								}}</Button>
								<strong>{{ selectedFile.name }}</strong>
							</div>
							<p
								v-if="contentPreview.requiredProjects.length && !selectedTarget"
								class="m-0 text-secondary"
							>
								{{
									formatMessage(messages.requiredProjectList, {
										ids: contentPreview.requiredProjects.join(', '),
									})
								}}
							</p>
							<p v-if="missingProjects.length" class="m-0 text-red" role="alert">
								{{
									formatMessage(messages.requiredProjects, {
										ids: missingProjects.join(', '),
									})
								}}
							</p>
							<p v-if="selectedTarget?.satisfiedProjects.length" class="m-0 text-secondary">
								{{
									formatMessage(messages.satisfiedProjects, {
										ids: selectedTarget.satisfiedProjects.join(', '),
									})
								}}
							</p>
							<p v-if="contentPreview.conflictingProjects.length" class="m-0 text-red" role="alert">
								{{
									formatMessage(messages.conflictingProjects, {
										ids: contentPreview.conflictingProjects.join(', '),
									})
								}}
							</p>
							<p
								v-if="!contentBlocked && !contentPreview.targets.length"
								class="m-0 text-secondary"
							>
								{{ formatMessage(messages.noTargets) }}
							</p>
							<label
								v-if="contentPreview.targets.length"
								class="flex flex-col gap-1 text-sm font-semibold"
							>
								{{ formatMessage(messages.selectTarget) }}
								<select
									v-model="targetId"
									class="picker"
									:disabled="!!installedPath"
									@change="run(loadApplicableWorlds)"
								>
									<option disabled value="">{{ formatMessage(messages.selectTarget) }}</option>
									<option
										v-for="target in contentPreview.targets"
										:key="target.id"
										:value="target.id"
									>
										{{ target.name }} ·
										{{
											formatMessage(messages.versionSummary, {
												version: target.gameVersion,
												loader: target.loader,
											})
										}}
									</option>
								</select>
							</label>
							<template v-if="category === 'datapack' && targetId">
								<label v-if="worlds.length" class="flex flex-col gap-1 text-sm font-semibold">
									{{ formatMessage(messages.chooseWorld) }}
									<select v-model="world" class="picker" :disabled="!!installedPath">
										<option disabled value="">{{ formatMessage(messages.chooseWorld) }}</option>
										<option v-for="entry in worlds" :key="entry.path" :value="entry.path">
											{{ entry.name }}
										</option>
									</select>
								</label>
								<p v-else class="m-0 text-secondary">{{ formatMessage(messages.noWorld) }}</p>
							</template>
							<p v-if="installedPath" class="m-0 break-words text-secondary" role="status">
								{{ formatMessage(messages.contentInstalled, { path: installedPath }) }}
							</p>
							<Button
								v-else-if="contentPreview.targets.length"
								type="colored"
								color="brand"
								class="w-fit"
								:disabled="!targetId || contentBlocked || (category === 'datapack' && !world)"
								@click="installSelectedContent"
							>
								<DownloadIcon />{{ formatMessage(messages.installContent) }}
							</Button>
						</template>
						<template v-else-if="project">
							<div class="flex flex-wrap items-center gap-3">
								<Button size="sm" @click="project = null">{{
									formatMessage(messages.previous)
								}}</Button>
								<strong>{{ project.name }}</strong>
							</div>
							<p v-if="files && !files.files.length" class="m-0 text-secondary">
								{{ formatMessage(messages.noFiles) }}
							</p>
							<div
								v-for="file in files?.files"
								:key="file.id"
								class="flex flex-wrap items-center justify-between gap-3 rounded-xl bg-surface-3 p-3"
							>
								<div class="flex min-w-0 flex-col gap-1">
									<strong class="break-words">{{ file.name }}</strong>
									<span class="text-sm text-secondary">{{ file.gameVersions.join(', ') }}</span>
								</div>
								<Button
									v-if="file.autoDownload"
									type="colored"
									color="brand"
									@click="chooseOnlineFile(file)"
									><DownloadIcon />{{
										formatMessage(category === 'modpack' ? messages.import : messages.selectTarget)
									}}</Button
								>
								<ButtonLink
									v-else-if="project.websiteUrl"
									:href="`${project.websiteUrl.replace(/\/$/, '')}/files/${file.id}`"
									target="_blank"
									size="sm"
									>{{ formatMessage(manualDownloadMessage) }}</ButtonLink
								>
								<span v-else class="text-sm text-secondary">{{
									formatMessage(manualDownloadMessage)
								}}</span>
							</div>
							<div v-if="filePage > 0 || files?.hasNext" class="flex gap-2">
								<Button
									size="sm"
									:disabled="filePage === 0"
									@click="loadFiles(project, filePage - 1)"
									>{{ formatMessage(messages.previous) }}</Button
								>
								<Button
									size="sm"
									:disabled="!files?.hasNext"
									@click="loadFiles(project, filePage + 1)"
									>{{ formatMessage(messages.next) }}</Button
								>
							</div>
						</template>
						<template v-else-if="results">
							<p v-if="!results.projects.length" class="m-0 text-secondary">
								{{
									formatMessage(category === 'modpack' ? messages.noResults : messages.noContent)
								}}
							</p>
							<div
								v-for="result in results.projects"
								:key="result.id"
								class="flex flex-wrap items-center justify-between gap-3 rounded-xl bg-surface-3 p-3"
							>
								<div class="min-w-0 flex-1">
									<strong class="break-words">{{ result.name }}</strong>
									<p class="mb-0 mt-1 text-sm text-secondary">{{ result.summary }}</p>
								</div>
								<Button @click="loadFiles(result, 0)">{{
									formatMessage(messages.chooseVersion)
								}}</Button>
							</div>
							<div v-if="resultPage > 0 || results.hasNext" class="flex gap-2">
								<Button
									size="sm"
									:disabled="resultPage === 0"
									@click="loadProjects(resultPage - 1)"
									>{{ formatMessage(messages.previous) }}</Button
								>
								<Button
									size="sm"
									:disabled="!results.hasNext"
									@click="loadProjects(resultPage + 1)"
									>{{ formatMessage(messages.next) }}</Button
								>
							</div>
						</template>
					</template>
				</section>
			</template>
		</fieldset>
	</main>
</template>

<style scoped>
.picker {
	appearance: auto;
	min-height: 2.5rem;
	border: 1px solid var(--color-button-bg);
	border-radius: 0.75rem;
	background: var(--color-bg);
	color: var(--color-contrast);
	padding: 0 0.75rem;
}
</style>
