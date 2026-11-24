<script setup lang="ts">
import {
	defineMessages,
	injectNotificationManager,
	Toggle,
	Combobox,
	type ComboboxOption,
	useSavable,
	useVIntl,
} from '@modrinth/ui'
import { useMutation, useQuery, useQueryClient } from '@tanstack/vue-query'
import { computed, onBeforeUnmount, onMounted } from 'vue'

import { useAppSettings } from '@/composables/use-app-settings.ts'
import { injectAppSettingsModal } from '@/features/settings/app-settings-modal/context'
import {
	type AppSettings,
	appSettingsKeys,
	appSettingsQueryOptions,
	get,
	set,
} from '@/helpers/settings.ts'

const appSettings = useAppSettings()
const { formatMessage } = useVIntl()
const { handleError } = injectNotificationManager()
const settingsModal = injectAppSettingsModal()
const queryClient = useQueryClient()

const messages = defineMessages({
	startupAndNavigationTitle: {
		id: 'app.behavior-settings.startup-and-navigation.title',
		defaultMessage: 'Startup and navigation',
	},
	contentTitle: {
		id: 'app.behavior-settings.content.title',
		defaultMessage: 'Home and content',
	},
	confirmationsTitle: {
		id: 'app.behavior-settings.confirmations.title',
		defaultMessage: 'Confirmations',
	},
	minimizeLauncherTitle: {
		id: 'app.appearance-settings.minimize-launcher.title',
		defaultMessage: 'Minimize app',
	},
	minimizeLauncherDescription: {
		id: 'app.appearance-settings.minimize-launcher.description',
		defaultMessage: 'Minimize ACL when Minecraft starts.',
	},
	refocusOnGameCloseTitle: {
		id: 'app.behavior-settings.refocus-on-game-close.title',
		defaultMessage: 'Refocus app when Minecraft closes',
	},
	refocusOnGameCloseDescription: {
		id: 'app.behavior-settings.refocus-on-game-close.description',
		defaultMessage: 'Bring ACL to the foreground when Minecraft exits.',
	},
	defaultLandingPageHome: {
		id: 'app.appearance-settings.default-landing-page.home',
		defaultMessage: 'Home',
	},
	defaultLandingPageLibrary: {
		id: 'app.appearance-settings.default-landing-page.library',
		defaultMessage: 'Library',
	},
	toggleSidebarTitle: {
		id: 'app.appearance-settings.toggle-sidebar.title',
		defaultMessage: 'Hide right sidebar',
	},
	toggleSidebarDescription: {
		id: 'app.appearance-settings.toggle-sidebar.description',
		defaultMessage: 'Hide the right sidebar by default and add a button to show or hide it.',
	},
	compactModeTitle: {
		id: 'app.appearance-settings.compact-mode.title',
		defaultMessage: 'Compact mode',
	},
	compactModeDescription: {
		id: 'app.appearance-settings.compact-mode.description',
		defaultMessage: 'Display library instances in a compact row layout.',
	},
	showPlayTimeTitle: {
		id: 'app.appearance-settings.show-play-time.title',
		defaultMessage: 'Show play time',
	},
	showPlayTimeDescription: {
		id: 'app.appearance-settings.show-play-time.description',
		defaultMessage: `Show how long you've played each instance.`,
	},
	hideNametagTitle: {
		id: 'app.appearance-settings.hide-nametag.title',
		defaultMessage: 'Hide nametag',
	},
	hideNametagDescription: {
		id: 'app.appearance-settings.hide-nametag.description',
		defaultMessage: 'Hide your username above the player preview on the Skin selector page.',
	},
	unknownPackWarningTitle: {
		id: 'app.appearance-settings.unknown-pack-warning.title',
		defaultMessage: 'Warn me before installing unknown modpacks',
	},
	unknownPackWarningDescription: {
		id: 'app.appearance-settings.unknown-pack-warning.description',
		defaultMessage:
			"Show a safety warning before installing a Modrinth Pack (.mrpack) that isn't hosted on Modrinth.",
	},
	skipNonEssentialWarningsTitle: {
		id: 'app.appearance-settings.skip-non-essential-warnings.title',
		defaultMessage: 'Skip non-essential warnings',
	},
	skipNonEssentialWarningsDescription: {
		id: 'app.appearance-settings.skip-non-essential-warnings.description',
		defaultMessage:
			'Skip confirmations for low-risk actions such as duplicate installs, normal content deletion, bulk updates, unlinking, and repairs. Warnings for dangerous actions are always shown.',
	},
	fileManagerDefaultTabTitle: {
		id: 'app.behavior-settings.file-manager-default-tab.title',
		defaultMessage: '文件管理默认打开项',
	},
	fileManagerDefaultTabDescription: {
		id: 'app.behavior-settings.file-manager-default-tab.description',
		defaultMessage: '进入实例文件管理时默认显示的分类。',
	},
	allFiles: { id: 'app.file-manager.category.all', defaultMessage: '全部文件' },
	mods: { id: 'app.file-manager.category.mods', defaultMessage: '模组' },
	resourcepacks: { id: 'app.file-manager.category.resourcepacks', defaultMessage: '资源包' },
	shaderpacks: { id: 'app.file-manager.category.shaderpacks', defaultMessage: '光影包' },
	schematics: { id: 'app.file-manager.category.schematics', defaultMessage: '投影原理图' },
})

type BehaviorSettingsState = {
	minimizeApp: boolean
	refocusOnGameClose: boolean
	hideRightSidebar: boolean
	compactInstanceCards: boolean
	showPlayTime: boolean
	hideNametag: boolean
	warnOnUnknownModpacks: boolean
	skipNonEssentialWarnings: boolean
	fileManagerDefaultTab: AppSettings['file_manager_default_tab']
}

const settingsQuery = useQuery(appSettingsQueryOptions())
await settingsQuery.suspense()

function getBehaviorSettingsState(settings: AppSettings): BehaviorSettingsState {
	return {
		minimizeApp: settings.hide_on_process_start,
		refocusOnGameClose: settings.refocus_on_game_close,
		compactInstanceCards: settings.compact_instance_cards,
		showPlayTime: settings.show_play_time,
		warnOnUnknownModpacks: settings.warn_on_unknown_modpacks,
		skipNonEssentialWarnings: settings.skip_non_essential_warnings,
		fileManagerDefaultTab: settings.file_manager_default_tab ?? 'all',
		hideRightSidebar: settings.toggle_sidebar,
		hideNametag: settings.hide_nametag_skins_page,
	}
}

const fileManagerTabOptions = computed<ComboboxOption<AppSettings['file_manager_default_tab']>[]>(() => [
	{ value: 'all', label: formatMessage(messages.allFiles) },
	{ value: 'mods', label: formatMessage(messages.mods) },
	{ value: 'resourcepacks', label: formatMessage(messages.resourcepacks) },
	{ value: 'shaderpacks', label: formatMessage(messages.shaderpacks) },
	{ value: 'schematics', label: formatMessage(messages.schematics) },
])

const settingsMutation = useMutation({
	mutationKey: appSettingsKeys.update,
	scope: { id: 'app-settings' },
	mutationFn: async (value: BehaviorSettingsState) => {
		const latestSettings = await get()
		const nextSettings: AppSettings = {
			...latestSettings,
			sync_behavior_across_devices: false,
			hide_on_process_start: value.minimizeApp,
			toggle_sidebar: value.hideRightSidebar,
			hide_nametag_skins_page: value.hideNametag,
			refocus_on_game_close: value.refocusOnGameClose,
			compact_instance_cards: value.compactInstanceCards,
			show_play_time: value.showPlayTime,
			warn_on_unknown_modpacks: value.warnOnUnknownModpacks,
			skip_non_essential_warnings: value.skipNonEssentialWarnings,
			file_manager_default_tab: value.fileManagerDefaultTab,
		}

		await set(nextSettings)
		queryClient.setQueryData(appSettingsKeys.all, nextSettings)
		appSettings.refocusOnGameClose = value.refocusOnGameClose
		appSettings.compactInstanceCards = value.compactInstanceCards
		appSettings.showPlayTime = value.showPlayTime
		appSettings.warnOnUnknownModpacks = value.warnOnUnknownModpacks
		appSettings.skipNonEssentialWarnings = value.skipNonEssentialWarnings
		appSettings.toggleSidebar = value.hideRightSidebar
		appSettings.hideNametagSkinsPage = value.hideNametag
	},
	onMutate: () => queryClient.cancelQueries({ queryKey: appSettingsKeys.all }),
	onError: handleError,
	onSettled: () => queryClient.invalidateQueries({ queryKey: appSettingsKeys.all }),
})

const { saved, current, changes, saving, hasChanges, reset, save } = useSavable(
	() => getBehaviorSettingsState(settingsQuery.data.value!),
	() => settingsMutation.mutateAsync({ ...current.value }),
)

async function saveBehaviorSettings(): Promise<void> {
	try {
		await save()
	} catch {
		return
	}
}

onMounted(() => {
	settingsModal.registerUnsavedChangesController({
		hasChanges: () => hasChanges.value,
		getOriginal: () => saved.value,
		getModified: () => changes.value,
		isSaving: () => saving.value,
		reset,
		save: saveBehaviorSettings,
	})
})

onBeforeUnmount(() => {
	settingsModal.registerUnsavedChangesController(null)
})
</script>
<template>
	<section>
		<h2 class="m-0 text-xl font-semibold text-contrast">
			{{ formatMessage(messages.startupAndNavigationTitle) }}
		</h2>
		<div class="mt-4 flex flex-col gap-6">
			<div class="flex items-center justify-between gap-4">
				<div>
					<h3 class="m-0 text-lg font-semibold text-contrast">
						{{ formatMessage(messages.minimizeLauncherTitle) }}
					</h3>
					<p class="m-0 mt-1">
						{{ formatMessage(messages.minimizeLauncherDescription) }}
					</p>
				</div>
				<Toggle id="minimize-launcher" v-model="current.minimizeApp" />
			</div>

			<div class="flex items-center justify-between gap-4">
				<div>
					<h3 id="refocus-on-game-close-label" class="m-0 text-lg font-semibold text-contrast">
						{{ formatMessage(messages.refocusOnGameCloseTitle) }}
					</h3>
					<p id="refocus-on-game-close-description" class="m-0 mt-1">
						{{ formatMessage(messages.refocusOnGameCloseDescription) }}
					</p>
				</div>
				<Toggle
					id="refocus-on-game-close"
					v-model="current.refocusOnGameClose"
					aria-labelledby="refocus-on-game-close-label"
					aria-describedby="refocus-on-game-close-description"
				/>
			</div>

			<div class="flex items-center justify-between gap-4">
				<div>
					<h3 id="file-manager-default-tab-label" class="m-0 text-lg font-semibold text-contrast">
						{{ formatMessage(messages.fileManagerDefaultTabTitle) }}
					</h3>
					<p id="file-manager-default-tab-description" class="m-0 mt-1">
						{{ formatMessage(messages.fileManagerDefaultTabDescription) }}
					</p>
				</div>
				<Combobox
					v-model="current.fileManagerDefaultTab"
					:options="fileManagerTabOptions"
					aria-labelledby="file-manager-default-tab-label"
					class="min-w-48"
				/>
			</div>

			<div class="flex items-center justify-between gap-4">
				<div>
					<h3 class="m-0 text-lg font-semibold text-contrast">
						{{ formatMessage(messages.toggleSidebarTitle) }}
					</h3>
					<p class="m-0 mt-1">{{ formatMessage(messages.toggleSidebarDescription) }}</p>
				</div>
				<Toggle id="toggle-sidebar" v-model="current.hideRightSidebar" />
			</div>
		</div>
	</section>

	<section class="mt-8 border-0 border-t border-solid border-divider pt-6">
		<h2 class="m-0 text-xl font-semibold text-contrast">
			{{ formatMessage(messages.contentTitle) }}
		</h2>
		<div class="mt-4 flex flex-col gap-6">
			<div class="flex items-center justify-between gap-4">
				<div>
					<h3 class="m-0 text-lg font-semibold text-contrast">
						{{ formatMessage(messages.compactModeTitle) }}
					</h3>
					<p class="m-0 mt-1">{{ formatMessage(messages.compactModeDescription) }}</p>
				</div>
				<Toggle id="compact-mode" v-model="current.compactInstanceCards" />
			</div>

			<div class="flex items-center justify-between gap-4">
				<div>
					<h3 class="m-0 text-lg font-semibold text-contrast">
						{{ formatMessage(messages.showPlayTimeTitle) }}
					</h3>
					<p class="m-0 mt-1">{{ formatMessage(messages.showPlayTimeDescription) }}</p>
				</div>
				<Toggle id="show-play-time" v-model="current.showPlayTime" />
			</div>

			<div class="flex items-center justify-between gap-4">
				<div>
					<h3 class="m-0 text-lg font-semibold text-contrast">
						{{ formatMessage(messages.hideNametagTitle) }}
					</h3>
					<p class="m-0 mt-1">{{ formatMessage(messages.hideNametagDescription) }}</p>
				</div>
				<Toggle id="hide-nametag-skins-page" v-model="current.hideNametag" />
			</div>
		</div>
	</section>

	<section class="mt-8 border-0 border-t border-solid border-divider pt-6">
		<h2 class="m-0 text-xl font-semibold text-contrast">
			{{ formatMessage(messages.confirmationsTitle) }}
		</h2>
		<div class="mt-4 flex flex-col gap-6">
			<div class="flex items-center justify-between gap-4">
				<div>
					<h3 class="m-0 text-lg font-semibold text-contrast">
						{{ formatMessage(messages.unknownPackWarningTitle) }}
					</h3>
					<p class="m-0 mt-1">
						{{ formatMessage(messages.unknownPackWarningDescription) }}
					</p>
				</div>
				<Toggle
					id="warn-before-installing-unknown-modpacks"
					v-model="current.warnOnUnknownModpacks"
				/>
			</div>

			<div class="flex items-center justify-between gap-4">
				<div>
					<h3 class="m-0 text-lg font-semibold text-contrast">
						{{ formatMessage(messages.skipNonEssentialWarningsTitle) }}
					</h3>
					<p class="m-0 mt-1">
						{{ formatMessage(messages.skipNonEssentialWarningsDescription) }}
					</p>
				</div>
				<Toggle id="skip-non-essential-warnings" v-model="current.skipNonEssentialWarnings" />
			</div>
		</div>
	</section>
</template>
