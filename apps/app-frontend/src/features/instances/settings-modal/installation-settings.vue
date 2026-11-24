<script setup lang="ts">
import type { Labrinth } from '@modrinth/api-client'
import { FolderArchiveIcon, SpinnerIcon } from '@modrinth/assets'
import {
	Button,
	Checkbox,
	commonMessages,
	defineMessages,
	formatLoaderLabel,
	injectFilePicker,
	injectNotificationManager,
	InstallationSettingsLayout,
	NewModal,
	provideInstallationSettings,
	useDebugLogger,
	useVIntl,
} from '@modrinth/ui'
import type { GameVersionTag, PlatformTag } from '@modrinth/utils'
import { useQuery, useQueryClient } from '@tanstack/vue-query'
import { save } from '@tauri-apps/plugin-dialog'
import { computed, onBeforeUnmount, ref, useTemplateRef } from 'vue'

import { useManagedContentPolicy } from '@/composables/instances/use-managed-content-policy'
import { useAppSettings } from '@/composables/use-app-settings.ts'
import { trackEvent } from '@/helpers/analytics'
import { get_project_versions, get_version } from '@/helpers/cache'
import {
	install_existing_instance,
	install_pack_to_existing_instance,
	wait_for_install_job,
} from '@/helpers/install'
import {
	edit,
	exportInstanceArchive,
	get_linked_modpack_info,
	previewInstanceVersionSwitch,
	switchInstanceVersion,
	unlink_shared_instance,
	update_managed_modrinth_version,
	update_repair_modrinth,
} from '@/helpers/instance'
import { get_loader_versions } from '@/helpers/metadata'
import { get_game_versions, get_loaders } from '@/helpers/tags'
import type { Manifest } from '@/helpers/types'
import { instanceKeys } from '@/pages/instance/query-options'
import { injectAppEvents } from '@/providers/app-events'
import { provideInstanceBackup } from '@/providers/instance-backup'

import { injectInstanceSettings } from './instance-settings-context.ts'
import OptiFineInstallationSettings from './optifine-installation-settings.vue'
import SharedInstanceInstallationSettingsControls from './shared-instance-installation-settings-controls.vue'

const { addNotification, handleError } = injectNotificationManager()
const appEvents = injectAppEvents()
const filePicker = injectFilePicker()
const { formatMessage } = useVIntl()
const queryClient = useQueryClient()
const debug = useDebugLogger('AppInstallationSettings')
const appSettings = useAppSettings()

const { instance, offline, isMinecraftServer, onUnlinked, closeModal } = injectInstanceSettings()
const managedContentPolicy = useManagedContentPolicy(instance)
const skipNonEssentialWarnings = computed(() => appSettings.skipNonEssentialWarnings)

debug('metadata load: start', {
	instanceId: instance.value.id,
	loader: instance.value.loader,
	gameVersion: instance.value.game_version,
	installStage: instance.value.install_stage,
})

function getSupportedModpackLoaders() {
	return get_loaders().then((value: PlatformTag[]) =>
		value
			.filter((item) => item.supported_project_types.includes('modpack') || item.name === 'vanilla')
			.sort((a, b) => (a.name === 'vanilla' ? -1 : b.name === 'vanilla' ? 1 : 0)),
	)
}

const fabricVersionsQuery = useQuery({
	queryKey: ['instance-settings', 'loader-versions', 'fabric'],
	queryFn: () => get_loader_versions('fabric') as Promise<Manifest>,
})
const forgeVersionsQuery = useQuery({
	queryKey: ['instance-settings', 'loader-versions', 'forge'],
	queryFn: () => get_loader_versions('forge') as Promise<Manifest>,
})
const quiltVersionsQuery = useQuery({
	queryKey: ['instance-settings', 'loader-versions', 'quilt'],
	queryFn: () => get_loader_versions('quilt') as Promise<Manifest>,
})
const neoforgeVersionsQuery = useQuery({
	queryKey: ['instance-settings', 'loader-versions', 'neo'],
	queryFn: () => get_loader_versions('neo') as Promise<Manifest>,
})
const gameVersionsQuery = useQuery({
	queryKey: ['instance-settings', 'game-versions'],
	queryFn: () => get_game_versions() as Promise<GameVersionTag[]>,
})
const loadersQuery = useQuery({
	queryKey: ['instance-settings', 'loaders', 'modpack'],
	queryFn: getSupportedModpackLoaders,
})

const metadataLoading = computed(() =>
	[
		fabricVersionsQuery,
		forgeVersionsQuery,
		quiltVersionsQuery,
		neoforgeVersionsQuery,
		gameVersionsQuery,
		loadersQuery,
	].some((query) => query.isLoading.value),
)

debug('metadata queries configured', {
	instanceId: instance.value.id,
	loader: instance.value.loader,
	gameVersion: instance.value.game_version,
})

const isModrinthLinkedModpack = computed(
	() =>
		instance.value.link?.type === 'modrinth_modpack' ||
		instance.value.link?.type === 'server_project_modpack' ||
		(instance.value.link?.type === 'shared_instance' &&
			!!instance.value.link.modpack_project_id &&
			!!instance.value.link.modpack_version_id),
)
const isImportedModpack = computed(() => instance.value.link?.type === 'imported_modpack')
const isSharedInstanceManagedModpack = managedContentPolicy.isManagedModpack
const canUnlinkSharedInstance = managedContentPolicy.canUnlink

const modpackInfoQuery = useQuery({
	queryKey: computed(() => ['linkedModpackInfo', instance.value.id]),
	queryFn: () => get_linked_modpack_info(instance.value.id, 'must_revalidate'),
	enabled: computed(() => isModrinthLinkedModpack.value && !offline),
})
const modpackInfo = modpackInfoQuery.data

const repairing = ref(false)
const reinstalling = ref(false)
const unlinkingSharedInstance = ref(false)
const installationSettingsBusy = computed(
	() =>
		instance.value.quarantined ||
		instance.value.install_stage !== 'installed' ||
		repairing.value ||
		reinstalling.value ||
		unlinkingSharedInstance.value ||
		!!offline,
)
const installationSettingsBusyMessage = computed(() =>
	instance.value.quarantined ? formatMessage(messages.locked) : null,
)

async function unlinkSharedInstance() {
	unlinkingSharedInstance.value = true
	try {
		await unlink_shared_instance(instance.value.id)
		await queryClient.invalidateQueries({
			queryKey: instanceKeys.sharedMembers(instance.value.id),
		})
		await queryClient.invalidateQueries({ queryKey: ['linkedModpackInfo', instance.value.id] })
		onUnlinked()
	} catch (error) {
		handleError(error)
	} finally {
		unlinkingSharedInstance.value = false
	}
}

const messages = defineMessages({
	loaderVersion: {
		id: 'instance.settings.tabs.installation.loader-version',
		defaultMessage: '{loader} version',
	},
	locked: {
		id: 'instance.settings.tabs.installation.locked',
		defaultMessage: 'Installation settings are unavailable while this instance is locked.',
	},
	switchTitle: {
		id: 'app.instance.version-switch.title',
		defaultMessage: 'Change Minecraft installation',
	},
	switchDescription: {
		id: 'app.instance.version-switch.description',
		defaultMessage:
			'Install Minecraft {version} with {loader}. Review these files before switching.',
	},
	switchNoIssues: {
		id: 'app.instance.version-switch.no-issues',
		defaultMessage: 'No compatibility issues were found in the indexed content.',
	},
	switchGameMismatch: {
		id: 'app.instance.version-switch.game-mismatch',
		defaultMessage: 'Requires a version that supports this Minecraft release',
	},
	switchLoaderMismatch: {
		id: 'app.instance.version-switch.loader-mismatch',
		defaultMessage: 'Requires a version that supports this mod loader',
	},
	switchUnknown: {
		id: 'app.instance.version-switch.unknown',
		defaultMessage: 'Compatibility requires a manual check',
	},
	switchAutoDisable: {
		id: 'app.instance.version-switch.auto-disable',
		defaultMessage: 'Disable incompatible mods that can be changed ({count})',
	},
	switchDone: {
		id: 'app.instance.version-switch.done',
		defaultMessage: 'Minecraft installation switched',
	},
	switchDisabledCount: {
		id: 'app.instance.version-switch.disabled-count',
		defaultMessage:
			'{count, plural, one {# incompatible mod disabled} other {# incompatible mods disabled}}',
	},
	saveSwitchBackup: {
		id: 'app.instance.version-switch.save-backup',
		defaultMessage: 'Save a backup before switching',
	},
	restoreSwitchBackup: {
		id: 'app.instance.version-switch.restore-backup',
		defaultMessage: 'You can restore the ZIP as a separate instance from the library.',
	},
	switchBackupSaved: {
		id: 'app.instance.version-switch.backup-saved',
		defaultMessage: 'Pre-switch backup saved',
	},
})

const versionSwitchModal = useTemplateRef<InstanceType<typeof NewModal>>('versionSwitchModal')
const versionSwitchItems = ref<Awaited<ReturnType<typeof previewInstanceVersionSwitch>>>([])
const versionSwitchTarget = ref({ version: '', loader: '' })
const disableConflicts = ref(true)
const savingSwitchBackup = ref(false)
const disableableMods = computed(() => versionSwitchItems.value.filter((item) => item.can_disable))
const issueMessage = {
	game_version: messages.switchGameMismatch,
	mod_loader: messages.switchLoaderMismatch,
	unverified: messages.switchUnknown,
}
let resolveVersionSwitch: ((accepted: boolean) => void) | undefined

function settleVersionSwitch(accepted: boolean) {
	const resolve = resolveVersionSwitch
	resolveVersionSwitch = undefined
	resolve?.(accepted)
}

function acceptVersionSwitch() {
	if (savingSwitchBackup.value) return
	settleVersionSwitch(true)
	versionSwitchModal.value?.hide()
}

async function saveVersionSwitchBackup() {
	if (savingSwitchBackup.value) return
	savingSwitchBackup.value = true
	try {
		const { id, name, game_version: version } = instance.value
		const fileName = `${name}-${version}`.replace(/[<>:"/\\|?*]/g, '_').replace(/[. ]+$/, '')
		const output = await save({
			defaultPath: `${fileName}.before-switch.acl-instance.zip`,
			filters: [{ name: 'ACL ZIP', extensions: ['zip'] }],
		})
		if (!output) return
		await exportInstanceArchive(id, output)
		addNotification({ type: 'success', title: formatMessage(messages.switchBackupSaved) })
	} catch (error) {
		handleError(error)
	} finally {
		savingSwitchBackup.value = false
	}
}

onBeforeUnmount(() => settleVersionSwitch(false))

function confirmVersionSwitch() {
	const modal = versionSwitchModal.value
	if (!modal) return Promise.resolve(false)
	return new Promise<boolean>((resolve) => {
		resolveVersionSwitch = resolve
		modal.show()
	})
}

function getManifest(loader: string) {
	const map: Record<string, Manifest | undefined> = {
		fabric: fabricVersionsQuery.data.value,
		forge: forgeVersionsQuery.data.value,
		quilt: quiltVersionsQuery.data.value,
		neoforge: neoforgeVersionsQuery.data.value,
	}
	const manifest = map[loader]
	debug('getManifest:', {
		loader,
		hasManifest: !!manifest,
		gameVersions: manifest?.gameVersions?.length ?? 0,
	})
	return manifest
}

async function installLocalModpackFromPicker() {
	const picked = await filePicker.pickModpackFile({ readFile: false })
	if (!picked?.path) return false

	const job = await install_pack_to_existing_instance(instance.value.id, {
		type: 'fromFile',
		path: picked.path,
	}).catch(handleError)
	if (!job) return false

	const completed = await wait_for_install_job(appEvents, job.job_id).catch(handleError)
	return !!completed
}

provideInstanceBackup(instance)

provideInstallationSettings({
	closeSettings: closeModal,
	loading: computed(() => metadataLoading.value || modpackInfoQuery.isLoading.value),
	installationInfo: computed(() => {
		const rows = [
			{
				label: formatMessage(commonMessages.platformLabel),
				value: formatLoaderLabel(instance.value.loader),
			},
			{
				label: formatMessage(commonMessages.gameVersionLabel),
				value: instance.value.game_version,
			},
		]
		if (instance.value.loader !== 'vanilla' && instance.value.loader_version) {
			rows.push({
				label: formatMessage(messages.loaderVersion, {
					loader: formatLoaderLabel(instance.value.loader),
				}),
				value: instance.value.loader_version,
			})
		}
		return rows
	}),
	isLinked: computed(
		() =>
			isModrinthLinkedModpack.value ||
			isImportedModpack.value ||
			instance.value.link?.type === 'server_project' ||
			isSharedInstanceManagedModpack.value,
	),
	isBusy: installationSettingsBusy,
	busyMessage: installationSettingsBusyMessage,
	skipNonEssentialWarnings,
	modpack: computed(() => {
		if (isImportedModpack.value && instance.value.link?.type === 'imported_modpack') {
			return {
				iconUrl: instance.value.icon_path,
				title: instance.value.link.name ?? instance.value.name,
				versionNumber: instance.value.link.version_number ?? undefined,
				filename: instance.value.link.filename ?? undefined,
			}
		}
		if (!modpackInfo.value) return null
		return {
			iconUrl: modpackInfo.value.project.icon_url,
			title: modpackInfo.value.project.title,
			link: `/project/${modpackInfo.value.project.slug ?? modpackInfo.value.project.id}`,
			versionNumber: modpackInfo.value.version?.version_number,
		}
	}),
	currentPlatform: computed(() => instance.value.loader),
	currentGameVersion: computed(() => instance.value.game_version),
	currentLoaderVersion: computed(() => instance.value.loader_version ?? ''),
	availablePlatforms: computed(() => loadersQuery.data.value?.map((x) => x.name) ?? []),

	resolveGameVersions(loader, showSnapshots) {
		const versions = gameVersionsQuery.data.value ?? []
		const filtered = versions.filter((item) => {
			if (loader === 'vanilla') return true
			const manifest = getManifest(loader)
			return !!manifest?.gameVersions?.some((x) => item.version === x.id)
		})
		const result = (
			showSnapshots ? filtered : filtered.filter((x) => x.version_type === 'release')
		).map((x) => ({ value: x.version, label: x.version }))
		debug('resolveGameVersions:', {
			loader,
			showSnapshots,
			totalVersions: versions.length,
			filteredVersions: filtered.length,
			resultVersions: result.length,
		})
		return result
	},

	resolveLoaderVersions(loader, gameVersion) {
		if (loader === 'vanilla' || !gameVersion) {
			debug('resolveLoaderVersions: skipped', { loader, gameVersion })
			return []
		}
		const manifest = getManifest(loader)
		if (!manifest) {
			debug('resolveLoaderVersions: no manifest', { loader, gameVersion })
			return []
		}
		const entry = manifest.gameVersions?.find((item) => item.id === gameVersion)
		if (entry?.versionGroup) {
			const result =
				manifest.versionGroups?.find((group) => group.id === entry.versionGroup)?.loaders ?? []
			debug('resolveLoaderVersions: version group result', {
				loader,
				gameVersion,
				versionGroup: entry.versionGroup,
				count: result.length,
			})
			return result
		}
		const placeholder = manifest.gameVersions?.find((item) => item.id === '${modrinth.gameVersion}')
		if (placeholder) {
			const result = manifest.gameVersions?.some((item) => item.id === gameVersion)
				? placeholder.loaders
				: []
			debug('resolveLoaderVersions: placeholder result', {
				loader,
				gameVersion,
				count: result.length,
			})
			return result
		}
		const result = entry?.loaders ?? []
		debug('resolveLoaderVersions: result', { loader, gameVersion, count: result.length })
		return result
	},

	resolveHasSnapshots(loader) {
		const versions = gameVersionsQuery.data.value ?? []
		if (loader === 'vanilla') {
			const result = versions.some((x) => x.version_type !== 'release')
			debug('resolveHasSnapshots: vanilla', { loader, result })
			return result
		}
		const manifest = getManifest(loader)
		const supported = versions.filter(
			(item) => !!manifest?.gameVersions?.some((x) => item.version === x.id),
		)
		const result = supported.some((x) => x.version_type !== 'release')
		debug('resolveHasSnapshots:', {
			loader,
			totalVersions: versions.length,
			supportedVersions: supported.length,
			result,
		})
		return result
	},

	async save(platform, gameVersion, loaderVersionId) {
		const instanceId = instance.value.id
		try {
			versionSwitchItems.value = await previewInstanceVersionSwitch(
				instanceId,
				gameVersion,
				platform,
			)
			versionSwitchTarget.value = { version: gameVersion, loader: formatLoaderLabel(platform) }
			disableConflicts.value = true
		} catch (error) {
			handleError(error)
			throw error
		}
		if (!(await confirmVersionSwitch()) || instance.value.id !== instanceId) {
			throw new Error('Version switch canceled')
		}
		const disableMods = disableConflicts.value ? disableableMods.value.map((item) => item.path) : []
		try {
			const job = await switchInstanceVersion(
				instanceId,
				gameVersion,
				platform,
				loaderVersionId,
				disableMods,
			)
			await wait_for_install_job(appEvents, job.job_id)
			addNotification({
				type: 'success',
				title: formatMessage(messages.switchDone),
				text: disableMods.length
					? formatMessage(messages.switchDisabledCount, { count: disableMods.length })
					: undefined,
			})
		} catch (error) {
			handleError(error)
			throw error
		} finally {
			await Promise.all([
				queryClient.invalidateQueries({ queryKey: instanceKeys.detail(instanceId) }),
				queryClient.invalidateQueries({ queryKey: instanceKeys.content(instanceId) }),
				queryClient.invalidateQueries({ queryKey: instanceKeys.fileManagerContent(instanceId) }),
			])
		}

		trackEvent('InstanceRepair', {
			loader: platform,
			game_version: gameVersion,
		})
	},

	async repair() {
		debug('repair: called', { instanceId: instance.value.id })
		repairing.value = true
		await install_existing_instance(instance.value.id, true).catch(handleError)
		repairing.value = false
		trackEvent('InstanceRepair', {
			loader: instance.value.loader,
			game_version: instance.value.game_version,
		})
		debug('repair: done')
	},

	async reinstallModpack() {
		debug('reinstallModpack: called', { instanceId: instance.value.id })
		reinstalling.value = true
		let shouldTrack = false
		try {
			if (isImportedModpack.value) {
				shouldTrack = await installLocalModpackFromPicker()
			} else {
				await update_repair_modrinth(instance.value.id).catch(handleError)
				shouldTrack = true
			}
		} finally {
			reinstalling.value = false
		}
		if (shouldTrack) {
			trackEvent('InstanceRepair', {
				loader: instance.value.loader,
				game_version: instance.value.game_version,
			})
		}
		debug('reinstallModpack: done')
	},

	async swapModpack() {
		debug('swapModpack: called', { instanceId: instance.value.id })
		reinstalling.value = true
		try {
			const installed = await installLocalModpackFromPicker()
			if (installed) {
				trackEvent('InstanceRepair', {
					loader: instance.value.loader,
					game_version: instance.value.game_version,
				})
			}
		} finally {
			reinstalling.value = false
		}
		debug('swapModpack: done')
	},

	async unlinkModpack() {
		debug('unlinkModpack: called', { instanceId: instance.value.id })
		await edit(instance.value.id, {
			link: null as unknown as undefined,
		})
		await queryClient.invalidateQueries({
			queryKey: ['linkedModpackInfo', instance.value.id],
		})
		onUnlinked()
		debug('unlinkModpack: done')
	},

	getCachedModpackVersions: () => null,
	async fetchModpackVersions() {
		debug('fetchModpackVersions: called', {
			projectId: instance.value.link?.project_id,
		})
		const versions = await get_project_versions(instance.value.link!.project_id!).catch(handleError)
		debug('fetchModpackVersions: done', { count: versions?.length ?? 0 })
		return (versions ?? []) as Labrinth.Versions.v2.Version[]
	},

	async getVersionChangelog(versionId: string) {
		debug('getVersionChangelog: called', { versionId })
		return (await get_version(versionId, 'must_revalidate').catch(
			() => null,
		)) as Labrinth.Versions.v2.Version | null
	},

	async onModpackVersionConfirm(version) {
		debug('onModpackVersionConfirm: called', {
			versionId: version.id,
			instanceId: instance.value.id,
		})
		await update_managed_modrinth_version(instance.value.id, version.id)
		await queryClient.invalidateQueries({
			queryKey: ['linkedModpackInfo', instance.value.id],
		})
		debug('onModpackVersionConfirm: done')
	},

	updaterModalProps: computed(() => ({
		isApp: true,
		currentVersionId: instance.value.link?.version_id ?? '',
		projectIconUrl: modpackInfo.value?.project?.icon_url,
		projectName: modpackInfo.value?.project?.title ?? 'Modpack',
		currentGameVersion: instance.value.game_version,
		currentLoader: instance.value.loader,
	})),

	isServer: false,
	isApp: true,
	showModpackVersionActions: computed(
		() =>
			isModrinthLinkedModpack.value &&
			!isMinecraftServer.value &&
			!isSharedInstanceManagedModpack.value,
	),
	isLocalFile: isImportedModpack,
	isManagedModpack: isSharedInstanceManagedModpack,
	managedModpackWarning: managedContentPolicy.managedModpackWarning,
	repairing,
	reinstalling,
})
</script>

<template>
	<NewModal
		ref="versionSwitchModal"
		:header="formatMessage(messages.switchTitle)"
		:on-hide="() => settleVersionSwitch(false)"
		:disable-close="savingSwitchBackup"
	>
		<div class="flex flex-col gap-3 sm:w-[30rem]">
			<p class="m-0">{{ formatMessage(messages.switchDescription, versionSwitchTarget) }}</p>
			<ul
				v-if="versionSwitchItems.length"
				class="m-0 flex max-h-64 flex-col gap-2 overflow-y-auto pl-5"
			>
				<li v-for="item in versionSwitchItems" :key="item.path" class="break-all">
					<span class="font-semibold text-contrast">{{ item.name }}</span>
					<span class="block text-sm">{{ formatMessage(issueMessage[item.issue]) }}</span>
				</li>
			</ul>
			<p v-else class="m-0">{{ formatMessage(messages.switchNoIssues) }}</p>
			<Checkbox
				v-if="disableableMods.length"
				v-model="disableConflicts"
				:label="formatMessage(messages.switchAutoDisable, { count: disableableMods.length })"
			/>
			<div class="flex flex-col items-start gap-1">
				<Button type="outlined" :disabled="savingSwitchBackup" @click="saveVersionSwitchBackup">
					<SpinnerIcon v-if="savingSwitchBackup" class="animate-spin" />
					<FolderArchiveIcon v-else />
					{{ formatMessage(messages.saveSwitchBackup) }}
				</Button>
				<p class="m-0 text-sm">{{ formatMessage(messages.restoreSwitchBackup) }}</p>
			</div>
		</div>
		<template #actions>
			<div class="flex flex-wrap justify-end gap-2">
				<Button :disabled="savingSwitchBackup" @click="versionSwitchModal?.hide()">
					{{ formatMessage(commonMessages.cancelButton) }}
				</Button>
				<Button
					type="colored"
					color="brand"
					:disabled="savingSwitchBackup"
					@click="acceptVersionSwitch"
				>
					{{ formatMessage(commonMessages.continueButton) }}
				</Button>
			</div>
		</template>
	</NewModal>
	<InstallationSettingsLayout>
		<template #extra>
			<SharedInstanceInstallationSettingsControls
				:can-unlink="canUnlinkSharedInstance"
				:busy="installationSettingsBusy"
				:unlinking="unlinkingSharedInstance"
				:unlink="unlinkSharedInstance"
			/>
			<OptiFineInstallationSettings v-if="!isMinecraftServer" :busy="installationSettingsBusy" />
		</template>
	</InstallationSettingsLayout>
</template>
