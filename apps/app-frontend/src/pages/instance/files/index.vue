<script setup lang="ts">
import { FileTextIcon, FolderOpenIcon, InfoIcon, PackageIcon, RefreshCwIcon, TrashIcon } from '@modrinth/assets'
import type { ContentItem, EditingFile, FileItem, UploadState } from '@modrinth/ui'
import {
	Button,
	commonMessages,
	defineMessages,
	FilePageLayout,
	NavTabs,
	injectNotificationManager,
	provideFileManager,
	ReadyTransition,
	useDebugLogger,
	useVIntl,
} from '@modrinth/ui'
import { useQuery, useQueryClient } from '@tanstack/vue-query'
import { invoke } from '@tauri-apps/api/core'
import { computed, ref, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'

import { useAppEvent } from '@/composables/use-app-event'
import { get_full_path } from '@/helpers/instance'
import { get_shared_instance_publish_preview, remove_project, toggle_disable_project } from '@/helpers/instance'
import { set_synced_pack_enabled } from '@/helpers/synced-packs'
import { get as getSettings } from '@/helpers/settings'
import { highlightInFolder } from '@/helpers/utils'
import ModalWrapper from '@/components/ui/modal/ModalWrapper.vue'
import ConfirmModalWrapper from '@/components/ui/modal/ConfirmModalWrapper.vue'
import SyncedContentModal from '@/components/ui/instance/SyncedContentModal.vue'
import { useManagedContentPolicy } from '@/composables/instances/use-managed-content-policy'
import { useSyncedPackActions } from '@/composables/instances/use-synced-pack-actions'

import { injectInstancePage } from '../instance-context'
import { instanceFileManagerContentQueryOptions, instanceKeys } from '../query-options'

const instancePage = injectInstancePage()
const instanceId = instancePage.instanceId
const instance = instancePage.instance
const isInstanceBusy = computed(() => instance.value.install_stage !== 'installed')
const queryClient = useQueryClient()
const managedContentPolicy = useManagedContentPolicy(computed(() => instance.value))
const { canMutateContent } = managedContentPolicy
const route = useRoute()
const router = useRouter()

const { formatMessage } = useVIntl()
const { addNotification } = injectNotificationManager()
const debug = useDebugLogger('Files')

const messages = defineMessages({
	readOnly: {
		id: 'instance.files.managed-content-read-only',
		defaultMessage: 'Manage installed mods from the Mods category',
	},
	saveAs: {
		id: 'instance.files.save-as',
		defaultMessage: 'Save as...',
	},
	addingFiles: {
		id: 'instance.files.adding-files',
		defaultMessage: 'Adding files ({completed}/{total})',
	},
	schematicDetails: { id: 'app.file-manager.schematic-details', defaultMessage: '原理图详情' },
	schematicDeleteTitle: { id: 'app.file-manager.schematic-delete-title', defaultMessage: '删除原理图？' },
	schematicDeleteDescription: {
		id: 'app.file-manager.schematic-delete-description',
		defaultMessage: '此操作会从当前实例的 schematics 文件夹中永久删除该文件。',
	},
	schematicName: { id: 'app.file-manager.schematic-name', defaultMessage: '名称' },
	schematicAuthor: { id: 'app.file-manager.schematic-author', defaultMessage: '作者' },
	schematicDimensions: { id: 'app.file-manager.schematic-dimensions', defaultMessage: '尺寸' },
	schematicBlocks: { id: 'app.file-manager.schematic-blocks', defaultMessage: '声明方块数' },
	notProvided: { id: 'app.file-manager.not-provided', defaultMessage: '未提供' },
	schematicLoadFailed: { id: 'app.file-manager.schematic-load-failed', defaultMessage: '读取原理图失败' },
	contentLoadFailed: { id: 'app.file-manager.content-load-failed', defaultMessage: '读取分类内容失败' },
	schematicDeleted: { id: 'app.file-manager.schematic-deleted', defaultMessage: '原理图已删除' },
	noSchematics: { id: 'app.file-manager.no-schematics', defaultMessage: '当前实例没有原理图文件' },
	contentDetails: { id: 'app.file-manager.content-details', defaultMessage: '文件详情' },
	contentName: { id: 'app.file-manager.content-name', defaultMessage: '名称' },
	contentVersion: { id: 'app.file-manager.content-version', defaultMessage: '版本' },
	contentFileName: { id: 'app.file-manager.content-file-name', defaultMessage: '文件名' },
	contentDescription: { id: 'app.file-manager.content-description', defaultMessage: '描述' },
	contentAuthors: { id: 'app.file-manager.content-authors', defaultMessage: '作者' },
	contentDeleteTitle: { id: 'app.file-manager.content-delete-title', defaultMessage: '删除此文件？' },
	contentDeleteDescription: { id: 'app.file-manager.content-delete-description', defaultMessage: '此操作会从当前实例中删除该内容文件。' },
	contentEnabled: { id: 'app.file-manager.content-enabled', defaultMessage: '停用' },
	contentDisabled: { id: 'app.file-manager.content-disabled', defaultMessage: '启用' },
	contentToggleFailed: { id: 'app.file-manager.content-toggle-failed', defaultMessage: '更改文件状态失败' },
	noCategoryContent: { id: 'app.file-manager.no-category-content', defaultMessage: '此分类中没有文件' },
	packFiles: { id: 'app.file-manager.pack-files', defaultMessage: '{count} 个项目' },
	packReadFailed: { id: 'app.file-manager.pack-read-failed', defaultMessage: '部分资源包目录读取失败' },
	modified: { id: 'app.file-manager.modified', defaultMessage: '修改时间' },
	contentSize: { id: 'app.file-manager.content-size', defaultMessage: '大小' },
	locate: { id: 'app.file-manager.locate', defaultMessage: '定位文件' },
	allFiles: { id: 'app.file-manager.category.all', defaultMessage: '全部文件' },
	mods: { id: 'app.file-manager.category.mods', defaultMessage: '模组' },
	resourcepacks: { id: 'app.file-manager.category.resourcepacks', defaultMessage: '资源包' },
	shaderpacks: { id: 'app.file-manager.category.shaderpacks', defaultMessage: '光影包' },
	schematics: { id: 'app.file-manager.category.schematics', defaultMessage: '投影原理图' },
})

type FileCategory = 'all' | 'mods' | 'resourcepacks' | 'shaderpacks' | 'schematics'
const validCategory = (value: unknown): value is FileCategory =>
	value === 'all' ||
	value === 'mods' ||
	value === 'resourcepacks' ||
	value === 'shaderpacks' ||
	value === 'schematics'
const initialCategory: FileCategory = validCategory(route.query.category)
	? route.query.category
	: (await getSettings()).file_manager_default_tab
if (!validCategory(route.query.category)) {
	void router.replace({ query: { ...route.query, category: initialCategory } })
}
const categoryPath: Record<FileCategory, string> = {
	all: '',
	mods: 'mods',
	resourcepacks: 'resourcepacks',
	shaderpacks: 'shaderpacks',
	schematics: 'schematics',
}
const selectedCategory = computed<FileCategory>(() =>
	validCategory(route.query.category) ? route.query.category : initialCategory,
)
const categoryTabs = computed(() =>
	([
		['all', 'allFiles'],
		['mods', 'mods'],
		['resourcepacks', 'resourcepacks'],
		['shaderpacks', 'shaderpacks'],
		['schematics', 'schematics'],
	] as const).map(([category, message]) => ({
		label: formatMessage(messages[message]),
		href: category,
	})),
)

const instanceRootQuery = useQuery(
	computed(() => ({
		queryKey: instanceKeys.rootPath(instancePage.instanceId.value),
		queryFn: () => get_full_path(instancePage.instanceId.value),
		enabled: !!instancePage.instanceId.value,
		staleTime: Infinity,
	})),
)
const instanceRoot = computed(() => instanceRootQuery.data.value ?? '')
const items = ref<FileItem[]>([])
/** True until the first directory read for the current instance path finishes (initial load only). */
const firstPaintPending = ref(true)
const loading = ref(true)
const error = ref<Error | null>(null)
const currentPath = ref(categoryPath[initialCategory])
const editingFile = ref<EditingFile | null>(null)
type SchematicFile = { name: string; path: string; extension: string; size: number; modified: number }
type SchematicDetails = { name: string | null; author: string | null; dimensions: string | null; declared_blocks: number | null }
type DirectoryPack = FileItem & { description?: string; icon_url?: string; pack_format?: number }
const schematics = ref<SchematicFile[]>([])
const directoryPacks = ref<DirectoryPack[]>([])
const loadingDirectoryPacks = ref(false)
const schematicDetails = ref<SchematicDetails | null>(null)
const selectedSchematic = ref<SchematicFile | null>(null)
const pendingDeleteSchematic = ref<{ instanceId: string; path: string } | null>(null)
const schematicDetailsModal = ref<InstanceType<typeof ModalWrapper> | null>(null)
const schematicDeleteModal = ref<InstanceType<typeof ConfirmModalWrapper> | null>(null)
const loadingSchematics = ref(false)
const deletingSchematic = ref(false)
const schematicListError = ref<Error | null>(null)
const directoryPackError = ref<Error | null>(null)
const contentDetailsItem = ref<ContentItem | null>(null)
const directoryPackDetails = ref<DirectoryPack | null>(null)
const pendingDeleteContentItem = ref<{ instanceId: string; item: ContentItem } | null>(null)
const pendingDeleteDirectoryPack = ref<{ instanceId: string; path: string } | null>(null)
const directoryPackRequest = ref(0)
const schematicRequest = ref(0)
const schematicDetailsRequest = ref(0)
const activeContentOperations = ref(new Set<string>())
const failedContentIcons = ref(new Set<string>())
const failedDirectoryIcons = ref(new Set<string>())
const contentDetailsModal = ref<InstanceType<typeof ModalWrapper> | null>(null)
const contentDeleteModal = ref<InstanceType<typeof ConfirmModalWrapper> | null>(null)
const syncedContentModal = ref<InstanceType<typeof SyncedContentModal>>()
const contentQuery = useQuery(computed(() => ({
	...instanceFileManagerContentQueryOptions(instanceId.value),
	enabled: !!instanceId.value,
})))
const refreshingCategory = ref(false)
const categoryContentItems = computed(() => {
	const categoryType = selectedCategory.value === 'mods' ? 'mod' : selectedCategory.value === 'resourcepacks' ? 'resourcepack' : 'shaderpack'
	return (contentQuery.data.value ?? []).filter((item) => item.project_type === categoryType)
})
function errorMessage(error: unknown, fallback: string): string {
	if (error instanceof Error && error.message.trim()) return error.message
	if (typeof error === 'string' && error.trim()) return error
	return fallback
}
const contentErrorMessage = computed(() => contentQuery.error.value
	? errorMessage(contentQuery.error.value, formatMessage(messages.contentLoadFailed))
	: '')
const categoryUsesDirectoryPacks = computed(() => selectedCategory.value === 'resourcepacks' || selectedCategory.value === 'shaderpacks')
const categoryLoading = computed(() => contentQuery.isFetching.value || (categoryUsesDirectoryPacks.value && loadingDirectoryPacks.value))
const categoryHasError = computed(() => Boolean(contentQuery.error.value || (categoryUsesDirectoryPacks.value && directoryPackError.value)))
const categoryHasRows = computed(() => categoryContentItems.value.length > 0 || (categoryUsesDirectoryPacks.value && directoryPacks.value.length > 0))
const contentRefresh = async () => {
	const targetId = instanceId.value
	await Promise.all([
		queryClient.invalidateQueries({ queryKey: instanceKeys.fileManagerContent(targetId) }),
		queryClient.invalidateQueries({ queryKey: instanceKeys.linkedContent(targetId) }),
		queryClient.invalidateQueries({ queryKey: instanceKeys.detail(targetId) }),
	])
}
const syncedPackActions = useSyncedPackActions(instance, syncedContentModal, canMutateContent, contentRefresh)

debug('setup: start, instance.id =', instanceId.value)

function isReadOnly(path: string): boolean {
	const normalized = path.startsWith('/') ? path.slice(1) : path
	return (
		normalized.split('/')[0].toLowerCase() === 'mods' ||
		items.value.some((item) => item.path === normalized && item.readOnly === true)
	)
}

async function writeBytes(path: string, bytes: Uint8Array, createOnly = false) {
	await invoke('plugin:files|file_write', {
		instanceId: instanceId.value,
		path,
		bytes: Array.from(bytes),
		createOnly,
	})
}

const directoryQuery = useQuery(
	computed(() => {
		const targetInstanceId = instancePage.instanceId.value
		const targetPath = currentPath.value
		return {
			queryKey: instanceKeys.files(targetInstanceId, targetPath),
			queryFn: () => invoke<FileItem[]>('plugin:files|file_list', { instanceId: targetInstanceId, path: targetPath }),
			enabled: !!instanceRoot.value && selectedCategory.value === 'all',
			staleTime: 30_000,
		}
	}),
)

watch(
	directoryQuery.data,
	(data) => {
		if (!data) return
		items.value = data
		firstPaintPending.value = false
	},
	{ immediate: true },
)
watch(directoryQuery.isFetching, (fetching) => {
	loading.value = fetching
})
watch(directoryQuery.error, (queryError) => {
	error.value = queryError
	if (queryError) items.value = []
})
watch(selectedCategory, (category) => {
	currentPath.value = categoryPath[category]
})

await instanceRootQuery.suspense()
if (selectedCategory.value === 'all') await directoryQuery.refetch()
firstPaintPending.value = false

async function refresh() {
	if (selectedCategory.value !== 'all') return
	debug('refresh: called, currentPath =', currentPath.value, 'instanceRoot =', instanceRoot.value)
	await directoryQuery.refetch()
}

async function refreshSelectedCategory() {
	if (refreshingCategory.value) return
	refreshingCategory.value = true
	try {
		if (selectedCategory.value === 'schematics') {
			await loadSchematics()
		} else if (selectedCategory.value === 'resourcepacks' || selectedCategory.value === 'shaderpacks') {
			await Promise.all([contentQuery.refetch(), loadDirectoryPacks(selectedCategory.value)])
		} else if (selectedCategory.value === 'mods') {
			await contentQuery.refetch()
		} else {
			await refresh()
		}
	} finally {
		refreshingCategory.value = false
	}
}

function navigateTo(path: string) {
	debug('navigateTo:', path)
	currentPath.value = path.startsWith('/') ? path.slice(1) : path
	refresh()
}

async function loadSchematics() {
	const targetInstanceId = instanceId.value
	const request = ++schematicRequest.value
	schematics.value = []
	loadingSchematics.value = true
	schematicListError.value = null
	try {
		const rows = await invoke<SchematicFile[]>('plugin:files|schematic_list', {
			instanceId: targetInstanceId,
		})
		if (instanceId.value === targetInstanceId && request === schematicRequest.value && selectedCategory.value === 'schematics') schematics.value = rows
	} catch (e) {
		if (instanceId.value === targetInstanceId && request === schematicRequest.value) schematicListError.value = new Error(errorMessage(e, formatMessage(messages.schematicLoadFailed)))
	} finally {
		if (instanceId.value === targetInstanceId && request === schematicRequest.value) loadingSchematics.value = false
	}
}

function packDescription(value: unknown): string | undefined {
	const text: string[] = []
	let visited = 0
	const visit = (part: unknown, depth = 0) => {
		visited += 1
		if (visited > 10_000 || depth > 48) return
		if (typeof part === 'string') text.push(part)
		else if (Array.isArray(part)) part.forEach((child) => visit(child, depth + 1))
		else if (part && typeof part === 'object') {
			const component = part as Record<string, unknown>
			if (component.text) visit(component.text, depth + 1)
			if (component.extra) visit(component.extra, depth + 1)
		}
	}
	visit(value)
	const description = text.join('').replace(/[\u0000-\u001f\u007f]/g, '').trim()
	return description ? [...description].slice(0, 4096).join('') : undefined
}

function bytesToBase64(bytes: number[]) {
	let binary = ''
	for (let index = 0; index < bytes.length; index += 0x8000) {
		binary += String.fromCharCode(...bytes.slice(index, index + 0x8000))
	}
	return btoa(binary)
}

async function loadDirectoryPacks(category: 'resourcepacks' | 'shaderpacks') {
	const targetInstanceId = instanceId.value
	const request = ++directoryPackRequest.value
	directoryPacks.value = []
	directoryPackError.value = null
	loadingDirectoryPacks.value = true
	try {
		const entries = await invoke<FileItem[]>('plugin:files|file_list', {
			instanceId: targetInstanceId,
			path: category,
		})
		const directories = entries.filter((entry) => entry.type === 'directory').slice(0, 500)
		const failures: Error[] = []
		const detected = await Promise.all(directories.map(async (directory) => {
			try {
				const children = await invoke<FileItem[]>('plugin:files|file_list', {
					instanceId: targetInstanceId,
					path: directory.path,
				})
				if (category === 'resourcepacks') {
					const meta = children.find((entry) => entry.type === 'file' && entry.name === 'pack.mcmeta')
				if (!meta || (meta.size ?? Number.POSITIVE_INFINITY) > 1024 * 1024) return null
					const raw = await invoke<number[]>('plugin:files|file_read', { instanceId: targetInstanceId, path: meta.path })
					if (raw.length > 1024 * 1024) return null
					const parsed = JSON.parse(new TextDecoder().decode(new Uint8Array(raw))) as { pack?: { description?: unknown; pack_format?: number } }
					const icon = children.find((entry) => entry.type === 'file' && entry.name === 'pack.png' && (entry.size ?? 0) <= 8 * 1024 * 1024)
					let icon_url: string | undefined
					if (icon) {
						try {
							const iconBytes = await invoke<number[]>('plugin:files|file_read', { instanceId: targetInstanceId, path: icon.path })
							const isPng = [137, 80, 78, 71, 13, 10, 26, 10].every((byte, index) => iconBytes[index] === byte)
							if (iconBytes.length <= 8 * 1024 * 1024 && isPng) icon_url = `data:image/png;base64,${bytesToBase64(iconBytes)}`
						} catch {
							icon_url = undefined
						}
					}
					return { ...directory, description: packDescription(parsed.pack?.description), icon_url, pack_format: parsed.pack?.pack_format }
				}
				return children.some((entry) => entry.name === 'shaders' && entry.type === 'directory') ? directory : null
			} catch (error) {
				failures.push(error instanceof Error ? error : new Error(String(error)))
				return null
			}
		}))
		if (instanceId.value === targetInstanceId && request === directoryPackRequest.value && selectedCategory.value === category) {
			directoryPacks.value = detected.filter((item): item is DirectoryPack => item !== null)
			if (failures.length) directoryPackError.value = new Error(`无法读取 ${failures.length} 个目录包`)
		}
	} catch (e) {
		if (instanceId.value === targetInstanceId && request === directoryPackRequest.value) {
			directoryPacks.value = []
			directoryPackError.value = e instanceof Error ? e : new Error(String(e))
		}
		debug('Could not inspect unpacked pack folders', e)
	} finally {
		if (instanceId.value === targetInstanceId && request === directoryPackRequest.value) loadingDirectoryPacks.value = false
	}
}

async function showSchematicDetails(schematic: SchematicFile) {
	const targetInstanceId = instanceId.value
	const request = ++schematicDetailsRequest.value
	const targetPath = schematic.path
	selectedSchematic.value = schematic
	schematicDetails.value = null
	try {
		const details = await invoke<SchematicDetails>('plugin:files|schematic_details', {
			instanceId: targetInstanceId,
			path: targetPath,
		})
		if (instanceId.value === targetInstanceId && selectedCategory.value === 'schematics' && request === schematicDetailsRequest.value && selectedSchematic.value?.path === targetPath) {
			schematicDetails.value = details
			schematicDetailsModal.value?.show()
		}
	} catch (e) {
		if (instanceId.value === targetInstanceId && selectedCategory.value === 'schematics' && request === schematicDetailsRequest.value && selectedSchematic.value?.path === targetPath) {
			addNotification({ title: formatMessage(messages.schematicLoadFailed), text: errorMessage(e, formatMessage(messages.schematicLoadFailed)), type: 'error' })
		}
	}
}

function requestDeleteSchematic(schematic: SchematicFile) {
	pendingDeleteSchematic.value = { instanceId: instanceId.value, path: schematic.path }
	schematicDeleteModal.value?.show()
}

async function deleteSchematic() {
	const target = pendingDeleteSchematic.value
	pendingDeleteSchematic.value = null
	if (!target || target.instanceId !== instanceId.value || isInstanceBusy.value || deletingSchematic.value) return
	const targetInstanceId = target.instanceId
	const targetPath = target.path
	deletingSchematic.value = true
	try {
		await invoke('plugin:files|schematic_delete', { instanceId: targetInstanceId, path: targetPath })
		if (instanceId.value === targetInstanceId) {
			schematics.value = schematics.value.filter((item) => item.path !== targetPath)
			addNotification({ title: formatMessage(messages.schematicDeleted), type: 'success' })
			await refresh()
		}
	} catch (e) {
		if (instanceId.value === targetInstanceId) {
			addNotification({ title: formatMessage(commonMessages.deleteFailedLabel), text: e instanceof Error ? e.message : '', type: 'error' })
		}
	}
	finally {
		deletingSchematic.value = false
	}
}

function isContentBusy(item: ContentItem) {
	return activeContentOperations.value.has(item.file_path ?? item.id)
}

function contentIconUrl(item: ContentItem): string | undefined {
	const candidates = [item.project?.icon_url, item.embedded_metadata?.icon_url]
		.filter((url): url is string => !!url)
		.filter((url, index, all) => all.indexOf(url) === index)
	return candidates.find((url) => !failedContentIcons.value.has(`${item.id}\0${url}`))
}

function markContentIconFailed(item: ContentItem) {
	const url = contentIconUrl(item)
	if (!url) return
	failedContentIcons.value = new Set([...failedContentIcons.value, `${item.id}\0${url}`])
}

function markDirectoryIconFailed(item: DirectoryPack) {
	failedDirectoryIcons.value = new Set([...failedDirectoryIcons.value, `${instanceId.value}\0${item.path}`])
}

async function locateContentPath(relativePath: string | undefined) {
	const targetInstanceId = instanceId.value
	if (!relativePath) return
	try {
		const rootPath = await get_full_path(targetInstanceId)
		if (instanceId.value !== targetInstanceId) return
		const absolutePath = `${rootPath.replace(/[\\/]+$/, '')}\\${relativePath.replace(/[\\/]/g, '\\')}`
		await highlightInFolder(absolutePath)
	} catch (error) {
		if (instanceId.value === targetInstanceId) {
			addNotification({ title: formatMessage(messages.locate), text: error instanceof Error ? error.message : '', type: 'error' })
		}
	}
}

async function refreshContentAndFiles(targetInstanceId: string) {
	await Promise.all([
		queryClient.invalidateQueries({ queryKey: instanceKeys.fileManagerContent(targetInstanceId) }),
		queryClient.invalidateQueries({ queryKey: instanceKeys.linkedContent(targetInstanceId) }),
		queryClient.invalidateQueries({ queryKey: instanceKeys.detail(targetInstanceId) }),
	])
	if (instanceId.value === targetInstanceId) await contentQuery.refetch()
}

async function toggleContent(item: ContentItem) {
	if (!item.file_path || isInstanceBusy.value || !canMutateContent(item) || isContentBusy(item)) return
	const targetInstanceId = instanceId.value
	const operationKey = item.file_path
	activeContentOperations.value = new Set([...activeContentOperations.value, operationKey])
	try {
		const syncOption = item.project_type === 'resourcepack' ? 'resource_packs' : 'data_packs'
		if (item.synced_pack && instance.value.synced_options[syncOption]) {
			await set_synced_pack_enabled(item.synced_pack.id, !item.enabled)
		} else {
			await toggle_disable_project(targetInstanceId, item.file_path, !item.enabled)
		}
		if (instanceId.value === targetInstanceId) {
			await refreshContentAndFiles(targetInstanceId)
			if (instance.value.shared_instance?.role === 'owner') {
				await get_shared_instance_publish_preview(targetInstanceId).catch((error) => debug('Could not reconcile shared content preview', error))
			}
		}
	} catch (error) {
		if (instanceId.value === targetInstanceId) {
			addNotification({ title: formatMessage(messages.contentToggleFailed), text: error instanceof Error ? error.message : '', type: 'error' })
		}
	} finally {
		const active = new Set(activeContentOperations.value)
		active.delete(operationKey)
		activeContentOperations.value = active
	}
}

function showContentDetails(item: ContentItem) {
	contentDetailsItem.value = item
	directoryPackDetails.value = null
	contentDetailsModal.value?.show()
}

function showDirectoryPackDetails(item: DirectoryPack) {
	directoryPackDetails.value = item
	contentDetailsItem.value = null
	contentDetailsModal.value?.show()
}

function requestDeleteContent(item: ContentItem) {
	if (isInstanceBusy.value || !canMutateContent(item) || isContentBusy(item)) return
	pendingDeleteDirectoryPack.value = null
	pendingDeleteContentItem.value = { instanceId: instanceId.value, item }
	contentDeleteModal.value?.show()
}

function requestDeleteDirectoryPack(item: DirectoryPack) {
	if (isInstanceBusy.value || item.readOnly) return
	pendingDeleteDirectoryPack.value = { instanceId: instanceId.value, path: item.path }
	pendingDeleteContentItem.value = null
	contentDeleteModal.value?.show()
}

async function deleteContent() {
	const directory = pendingDeleteDirectoryPack.value
	const pendingItem = pendingDeleteContentItem.value
	// Consume the modal target up front so a later confirmation cannot reuse it.
	pendingDeleteDirectoryPack.value = null
	pendingDeleteContentItem.value = null
	if (directory) {
		if (directory.instanceId !== instanceId.value || isInstanceBusy.value) return
		const targetInstanceId = directory.instanceId
		try {
			await invoke('plugin:files|file_delete', {
				instanceId: targetInstanceId,
				path: directory.path,
				recursive: true,
			})
			if (instanceId.value === targetInstanceId) {
				directoryPacks.value = directoryPacks.value.filter((item) => item.path !== directory.path)
				await refreshContentAndFiles(targetInstanceId)
				if (selectedCategory.value === 'resourcepacks' || selectedCategory.value === 'shaderpacks') {
					await loadDirectoryPacks(selectedCategory.value)
				}
			}
		} catch (error) {
			if (instanceId.value === targetInstanceId) {
				addNotification({ title: formatMessage(commonMessages.deleteFailedLabel), text: error instanceof Error ? error.message : '', type: 'error' })
			}
		}
		return
	}
	if (!pendingItem || pendingItem.instanceId !== instanceId.value) return
	const item = pendingItem.item
	const targetInstanceId = pendingItem.instanceId
	if (!item?.file_path || isInstanceBusy.value || !canMutateContent(item) || isContentBusy(item)) return
	const operationKey = item.file_path
	activeContentOperations.value = new Set([...activeContentOperations.value, operationKey])
	try {
		const confirmedSyncedAction = await syncedPackActions.confirmDeleteItems([item])
		if (confirmedSyncedAction === false) return
		if (!(await syncedPackActions.deleteSyncedItem(item))) {
			await remove_project(targetInstanceId, item.file_path)
		}
		if (instanceId.value === targetInstanceId) {
			await refreshContentAndFiles(targetInstanceId)
			if (instance.value.shared_instance?.role === 'owner') {
				await get_shared_instance_publish_preview(targetInstanceId).catch((error) => debug('Could not reconcile shared content preview', error))
			}
		}
	} catch (error) {
		if (instanceId.value === targetInstanceId) {
			addNotification({ title: formatMessage(commonMessages.deleteFailedLabel), text: error instanceof Error ? error.message : '', type: 'error' })
		}
	} finally {
		const active = new Set(activeContentOperations.value)
		active.delete(operationKey)
		activeContentOperations.value = active
	}
}

watch([selectedCategory, instanceId], ([category]) => {
	if (category === 'schematics') void loadSchematics()
	else {
		schematicRequest.value += 1
		schematicDetailsRequest.value += 1
		loadingSchematics.value = false
		schematics.value = []
		schematicListError.value = null
	}
	if (category === 'resourcepacks' || category === 'shaderpacks') void loadDirectoryPacks(category)
	else {
		directoryPackRequest.value += 1
		loadingDirectoryPacks.value = false
		directoryPacks.value = []
		directoryPackError.value = null
	}
}, { immediate: true })

function startEditing(file: EditingFile) {
	editingFile.value = file
}

function stopEditing() {
	editingFile.value = null
}

async function handleCreateItem(name: string, type: 'file' | 'directory') {
	const targetPath = currentPath.value ? `${currentPath.value}/${name}` : name
	try {
		if (type === 'directory') {
			await invoke('plugin:files|file_create_directory', {
				instanceId: instanceId.value,
				path: targetPath,
			})
		} else {
			await writeBytes(targetPath, new Uint8Array(), true)
		}
		await refresh()
	} catch (e) {
		addNotification({
			title: formatMessage(commonMessages.createFailedLabel),
			text: e instanceof Error ? e.message : '',
			type: 'error',
		})
	}
}

async function handleRenameItem(path: string, newName: string) {
	const parentDir = path.includes('/') ? path.substring(0, path.lastIndexOf('/')) : ''
	const newPath = parentDir ? `${parentDir}/${newName}` : newName
	try {
		await invoke('plugin:files|file_rename', {
			instanceId: instanceId.value,
			source: path,
			destination: newPath,
		})
		await refresh()
	} catch (e) {
		addNotification({
			title: formatMessage(commonMessages.renameFailedLabel),
			text: e instanceof Error ? e.message : '',
			type: 'error',
		})
	}
}

async function handleMoveItem(source: string, destination: string) {
	try {
		await invoke('plugin:files|file_rename', { instanceId: instanceId.value, source, destination })
		await refresh()
	} catch (e) {
		addNotification({
			title: formatMessage(commonMessages.moveFailedLabel),
			text: e instanceof Error ? e.message : '',
			type: 'error',
		})
	}
}

async function handleDeleteItem(path: string, recursive: boolean) {
	try {
		await invoke('plugin:files|file_delete', { instanceId: instanceId.value, path, recursive })
		await refresh()
	} catch (e) {
		addNotification({
			title: formatMessage(commonMessages.deleteFailedLabel),
			text: e instanceof Error ? e.message : '',
			type: 'error',
		})
	}
}

async function handleReadFile(path: string): Promise<string> {
	const bytes = await invoke<number[]>('plugin:files|file_read', {
		instanceId: instanceId.value,
		path,
	})
	return new TextDecoder().decode(new Uint8Array(bytes))
}

async function handleReadFileAsBlob(path: string): Promise<Blob> {
	const bytes = await invoke<number[]>('plugin:files|file_read', {
		instanceId: instanceId.value,
		path,
	})
	return new Blob([new Uint8Array(bytes)])
}

async function handleWriteFile(path: string, content: string) {
	await writeBytes(path, new TextEncoder().encode(content))
}

async function handleDownloadFile(path: string, _fileName: string) {
	await invoke('plugin:files|file_save_as', {
		instanceId: instanceId.value,
		filePath: path,
	})
}

const uploadState = ref<UploadState>({
	isUploading: false,
	currentFileName: null,
	currentFileProgress: 0,
	uploadedBytes: 0,
	totalBytes: 0,
	completedFiles: 0,
	totalFiles: 0,
})

async function handleUploadFiles(files: File[]) {
	if (files.length === 0) return

	uploadState.value = {
		isUploading: true,
		currentFileName: '',
		currentFileProgress: 0,
		uploadedBytes: 0,
		totalBytes: files.reduce((sum, f) => sum + f.size, 0),
		completedFiles: 0,
		totalFiles: files.length,
	}
	try {
		for (const file of files) {
			uploadState.value.currentFileName = file.name
			const buffer = await file.arrayBuffer()
			const targetPath = currentPath.value ? `${currentPath.value}/${file.name}` : file.name
			await writeBytes(targetPath, new Uint8Array(buffer))
			uploadState.value.completedFiles++
			uploadState.value.uploadedBytes += file.size
			uploadState.value.currentFileProgress = 1
		}
	} catch (e) {
		addNotification({
			title: formatMessage(commonMessages.uploadFailedLabel),
			text: e instanceof Error ? e.message : '',
			type: 'error',
		})
	} finally {
		uploadState.value.isUploading = false
		await refresh()
	}
}

async function handleExtractFile(path: string, override: boolean, dry: boolean) {
	try {
		return await invoke('plugin:files|file_extract_zip', {
			instanceId: instanceId.value,
			filePath: path,
			overrideConflicts: override,
			dryRun: dry,
		})
	} catch (e) {
		addNotification({
			title: formatMessage(commonMessages.extractFailedLabel),
			text: e instanceof Error ? e.message : '',
			type: 'error',
		})
	}
}

useAppEvent('instance', async (event) => {
	debug('app event: instance =', event.event, 'path =', event.instance_id)
	if (event.instance_id === instanceId.value && event.event === 'synced') {
		debug('app event: synced instance matched, calling refresh')
		await refreshSelectedCategory()
	}
})

watch(instanceId, async () => {
	debug('watch instance.id: changed to', instanceId.value)
	firstPaintPending.value = true
	currentPath.value = categoryPath[selectedCategory.value]
	await instanceRootQuery.refetch()
	if (selectedCategory.value === 'all') await refresh()
})

provideFileManager({
	isReadOnly,
	readOnlyReason: computed(() => formatMessage(messages.readOnly)),
	items,
	loading,
	error,
	currentPath,
	navigateTo,
	editingFile,
	startEditing,
	stopEditing,
	createItem: handleCreateItem,
	renameItem: handleRenameItem,
	moveItem: handleMoveItem,
	deleteItem: handleDeleteItem,
	readFile: handleReadFile,
	readFileAsBlob: handleReadFileAsBlob,
	writeFile: handleWriteFile,
	downloadFile: handleDownloadFile,
	uploadFiles: handleUploadFiles,
	uploadState,
	extractFile: handleExtractFile,
	refresh,
	basePath: instanceRoot,
	openInFolder: (path: string) => highlightInFolder(path),
	downloadButtonLabel: formatMessage(messages.saveAs),
	uploadingLabel: (completed: number, total: number) =>
		formatMessage(messages.addingFiles, { completed, total }),
})
</script>

<template>
	<ReadyTransition :pending="firstPaintPending">
		<div>
			<NavTabs :links="categoryTabs" query="category" class="mb-4" />
			<div v-if="selectedCategory !== 'all'" class="mb-4 flex justify-end">
				<Button
					:disabled="refreshingCategory"
					:aria-label="formatMessage(commonMessages.refreshButton)"
					@click="refreshSelectedCategory"
				>
					<RefreshCwIcon aria-hidden="true" />
					{{ formatMessage(commonMessages.refreshButton) }}
				</Button>
			</div>
			<section v-if="selectedCategory === 'schematics'" class="flex flex-col gap-3">
				<p v-if="loadingSchematics" class="text-secondary">{{ formatMessage(commonMessages.loadingLabel) }}</p>
				<p v-if="schematicListError" class="text-red">{{ errorMessage(schematicListError, formatMessage(messages.schematicLoadFailed)) }}</p>
				<p v-if="!loadingSchematics && !schematicListError && schematics.length === 0" class="text-secondary">{{ formatMessage(messages.noSchematics) }}</p>
				<article
					v-for="schematic in schematics"
					:key="schematic.path"
					class="flex items-center gap-4 rounded-lg border border-solid border-divider bg-surface-2 p-4"
				>
					<FileTextIcon class="size-6 shrink-0 text-secondary" />
					<div class="min-w-0 flex-1">
						<p class="m-0 truncate font-semibold text-contrast">{{ schematic.name }}</p>
						<p class="m-0 truncate text-sm text-secondary">
							{{ schematic.path }} · {{ schematic.extension.toUpperCase() }} · {{ schematic.size }} B
						</p>
						<p class="m-0 text-xs text-secondary">{{ new Date(schematic.modified * 1000).toLocaleString() }}</p>
					</div>
					<Button v-tooltip="formatMessage(messages.schematicDetails)" :disabled="isInstanceBusy" :aria-label="formatMessage(messages.schematicDetails)" @click="showSchematicDetails(schematic)">
						<InfoIcon aria-hidden="true" />
					</Button>
					<Button v-tooltip="formatMessage(messages.locate)" :disabled="isInstanceBusy" :aria-label="formatMessage(messages.locate)" @click="locateContentPath(`schematics/${schematic.path}`)">
						<FolderOpenIcon aria-hidden="true" />
					</Button>
					<Button v-tooltip="formatMessage(commonMessages.deleteLabel)" :disabled="isInstanceBusy || deletingSchematic" :aria-label="formatMessage(commonMessages.deleteLabel)" @click="requestDeleteSchematic(schematic)">
						<TrashIcon aria-hidden="true" />
					</Button>
				</article>
			</section>
			<section v-else-if="selectedCategory !== 'all'" class="flex flex-col gap-3">
				<p v-if="categoryLoading" class="text-secondary">{{ formatMessage(commonMessages.loadingLabel) }}</p>
				<p v-if="contentErrorMessage" class="text-red">{{ contentErrorMessage }}</p>
				<p v-if="categoryUsesDirectoryPacks && directoryPackError" class="text-red">{{ errorMessage(directoryPackError, formatMessage(messages.packReadFailed)) }}</p>
				<p v-if="!categoryLoading && !categoryHasError && !categoryHasRows" class="text-secondary">{{ formatMessage(messages.noCategoryContent) }}</p>
				<article
					v-if="categoryContentItems.length > 0"
					v-for="item in categoryContentItems"
					:key="item.id"
					class="flex items-center gap-4 rounded-lg border border-solid border-divider bg-surface-2 p-4"
				>
					<img
						v-if="contentIconUrl(item)"
						:src="contentIconUrl(item)"
						alt=""
						class="size-10 shrink-0 rounded-md object-cover"
						@error="markContentIconFailed(item)"
					/>
					<PackageIcon v-else class="size-10 shrink-0 text-secondary" />
					<div class="min-w-0 flex-1">
						<p class="m-0 truncate font-semibold text-contrast">
							{{ item.project?.title || item.embedded_metadata?.name || item.file_name }}
						</p>
						<p class="m-0 truncate text-sm text-secondary">
							{{ item.version?.version_number || item.embedded_metadata?.version || item.file_name }}
						</p>
						<p v-if="item.embedded_metadata?.description" class="m-0 truncate text-sm text-secondary">
							{{ item.embedded_metadata.description }}
						</p>
						<p v-else class="m-0 truncate text-sm text-secondary">{{ item.file_name }}</p>
						<p class="m-0 text-xs text-secondary">
							{{ item.size ?? 0 }} B<span v-if="item.date_added"> · {{ new Date(item.date_added).toLocaleString() }}</span>
						</p>
					</div>
					<Button
						v-tooltip="formatMessage(messages.contentDetails)"
						:disabled="isInstanceBusy"
						:aria-label="formatMessage(messages.contentDetails)"
						@click="showContentDetails(item)"
					>
						<InfoIcon aria-hidden="true" />
					</Button>
					<Button
						v-tooltip="formatMessage(messages.locate)"
						:disabled="isInstanceBusy || !item.file_path"
						:aria-label="formatMessage(messages.locate)"
						@click="locateContentPath(item.file_path)"
					>
						<FolderOpenIcon aria-hidden="true" />
					</Button>
					<Button
						v-if="selectedCategory === 'mods'"
						:disabled="isInstanceBusy || !canMutateContent(item) || isContentBusy(item)"
						@click="toggleContent(item)"
					>
						{{ formatMessage(item.enabled ? messages.contentEnabled : messages.contentDisabled) }}
					</Button>
					<Button
						v-tooltip="formatMessage(commonMessages.deleteLabel)"
						:disabled="isInstanceBusy || !canMutateContent(item) || isContentBusy(item)"
						:aria-label="formatMessage(commonMessages.deleteLabel)"
						@click="requestDeleteContent(item)"
					>
						<TrashIcon aria-hidden="true" />
					</Button>
				</article>
				<article
					v-if="categoryUsesDirectoryPacks && directoryPacks.length > 0"
					v-for="item in directoryPacks"
					:key="item.path"
					class="flex items-center gap-4 rounded-lg border border-solid border-divider bg-surface-2 p-4"
				>
					<img
						v-if="item.icon_url && !failedDirectoryIcons.has(`${instanceId}\0${item.path}`)"
						:src="item.icon_url"
						alt=""
						class="size-10 shrink-0 rounded-md object-cover"
						@error="markDirectoryIconFailed(item)"
					/>
					<PackageIcon v-else class="size-10 shrink-0 text-secondary" />
					<div class="min-w-0 flex-1">
						<p class="m-0 truncate font-semibold text-contrast">{{ item.name }}</p>
						<p class="m-0 truncate text-sm text-secondary">
							{{ item.path }} · {{ formatMessage(messages.packFiles, { count: item.count ?? 0 }) }}
						</p>
						<p v-if="item.pack_format" class="m-0 truncate text-sm text-secondary">pack_format {{ item.pack_format }}</p>
						<p v-if="item.description" class="m-0 truncate text-sm text-secondary">{{ item.description }}</p>
						<p class="m-0 text-xs text-secondary">{{ new Date(item.modified * 1000).toLocaleString() }}</p>
					</div>
					<Button v-tooltip="formatMessage(messages.contentDetails)" :disabled="isInstanceBusy" :aria-label="formatMessage(messages.contentDetails)" @click="showDirectoryPackDetails(item)">
						<InfoIcon aria-hidden="true" />
					</Button>
					<Button v-tooltip="formatMessage(messages.locate)" :disabled="isInstanceBusy" :aria-label="formatMessage(messages.locate)" @click="locateContentPath(item.path)">
						<FolderOpenIcon aria-hidden="true" />
					</Button>
					<Button
						v-tooltip="formatMessage(commonMessages.deleteLabel)"
						:disabled="isInstanceBusy || item.readOnly"
						:aria-label="formatMessage(commonMessages.deleteLabel)"
						@click="requestDeleteDirectoryPack(item)"
					>
						<TrashIcon aria-hidden="true" />
					</Button>
				</article>
			</section>
			<FilePageLayout v-else :show-refresh-button="true" />
			<ModalWrapper ref="schematicDetailsModal" :header="formatMessage(messages.schematicDetails)">
				<dl v-if="schematicDetails" class="grid grid-cols-[auto_1fr] gap-x-5 gap-y-3">
					<dt>{{ formatMessage(messages.schematicName) }}</dt><dd>{{ schematicDetails.name || formatMessage(messages.notProvided) }}</dd>
					<dt>{{ formatMessage(messages.schematicAuthor) }}</dt><dd>{{ schematicDetails.author || formatMessage(messages.notProvided) }}</dd>
					<dt>{{ formatMessage(messages.schematicDimensions) }}</dt><dd>{{ schematicDetails.dimensions || formatMessage(messages.notProvided) }}</dd>
					<dt>{{ formatMessage(messages.schematicBlocks) }}</dt><dd>{{ schematicDetails.declared_blocks ?? formatMessage(messages.notProvided) }}</dd>
				</dl>
			</ModalWrapper>
			<ModalWrapper ref="contentDetailsModal" :header="formatMessage(messages.contentDetails)">
				<dl v-if="contentDetailsItem" class="grid grid-cols-[auto_1fr] gap-x-5 gap-y-3">
					<dt>{{ formatMessage(messages.contentName) }}</dt>
					<dd>{{ contentDetailsItem.project?.title || contentDetailsItem.embedded_metadata?.name || contentDetailsItem.file_name }}</dd>
					<dt>{{ formatMessage(messages.contentVersion) }}</dt>
					<dd>{{ contentDetailsItem.version?.version_number || contentDetailsItem.embedded_metadata?.version || formatMessage(messages.notProvided) }}</dd>
					<dt>{{ formatMessage(messages.contentFileName) }}</dt><dd>{{ contentDetailsItem.file_name }}</dd>
					<dt>{{ formatMessage(messages.contentDescription) }}</dt>
					<dd>{{ contentDetailsItem.embedded_metadata?.description || formatMessage(messages.notProvided) }}</dd>
					<dt>{{ formatMessage(messages.contentAuthors) }}</dt>
					<dd>{{ contentDetailsItem.embedded_metadata?.authors?.join(', ') || formatMessage(messages.notProvided) }}</dd>
					<dt>{{ formatMessage(messages.contentSize) }}</dt><dd>{{ contentDetailsItem.size ?? 0 }} B</dd>
					<dt>{{ formatMessage(messages.modified) }}</dt>
					<dd>{{ contentDetailsItem.date_added ? new Date(contentDetailsItem.date_added).toLocaleString() : formatMessage(messages.notProvided) }}</dd>
				</dl>
				<dl v-if="directoryPackDetails" class="grid grid-cols-[auto_1fr] gap-x-5 gap-y-3">
					<dt>{{ formatMessage(messages.contentName) }}</dt><dd>{{ directoryPackDetails.name }}</dd>
					<dt>{{ formatMessage(messages.contentFileName) }}</dt><dd>{{ directoryPackDetails.path }}</dd>
					<dt>{{ formatMessage(messages.contentDescription) }}</dt><dd>{{ directoryPackDetails.description || formatMessage(messages.notProvided) }}</dd>
					<dt>{{ formatMessage(messages.contentVersion) }}</dt><dd>{{ directoryPackDetails.pack_format ?? formatMessage(messages.notProvided) }}</dd>
					<dt>{{ formatMessage(messages.modified) }}</dt><dd>{{ new Date(directoryPackDetails.modified * 1000).toLocaleString() }}</dd>
				</dl>
			</ModalWrapper>
			<ConfirmModalWrapper
				ref="schematicDeleteModal"
				:title="formatMessage(messages.schematicDeleteTitle)"
				:description="formatMessage(messages.schematicDeleteDescription)"
				:proceed-label="formatMessage(commonMessages.deleteLabel)"
				@proceed="deleteSchematic"
			/>
			<ConfirmModalWrapper
				ref="contentDeleteModal"
				:title="formatMessage(messages.contentDeleteTitle)"
				:description="formatMessage(messages.contentDeleteDescription)"
				:proceed-label="formatMessage(commonMessages.deleteLabel)"
				@proceed="deleteContent"
			/>
			<SyncedContentModal ref="syncedContentModal" />
		</div>
	</ReadyTransition>
</template>
