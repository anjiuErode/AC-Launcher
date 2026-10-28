<script setup lang="ts">
import { Button, defineMessages, ProgressBar, useVIntl } from '@modrinth/ui'
import { getVersion } from '@tauri-apps/api/app'
import { invoke } from '@tauri-apps/api/core'
import { check, type Update } from '@tauri-apps/plugin-updater'
import { onMounted, ref } from 'vue'

const { formatMessage } = useVIntl()

const messages = defineMessages({
	appName: {
		id: 'app.settings.about.app-name',
		defaultMessage: 'ACL Launcher',
	},
	appSubtitle: {
		id: 'app.settings.about.app-subtitle',
		defaultMessage: 'AnjiuCraftLauncher · built for the Yingzhou guild',
	},
	versionTitle: {
		id: 'app.settings.about.version-title',
		defaultMessage: 'Version',
	},
	checkUpdate: {
		id: 'app.settings.about.check-update',
		defaultMessage: 'Check for updates',
	},
	checking: {
		id: 'app.settings.about.checking',
		defaultMessage: 'Checking…',
	},
	upToDate: {
		id: 'app.settings.about.up-to-date',
		defaultMessage: 'You are on the latest version.',
	},
	updateFound: {
		id: 'app.settings.about.update-found',
		defaultMessage: 'Version {version} is available (released {date}).',
	},
	updateDisabled: {
		id: 'app.settings.about.update-disabled',
		defaultMessage: 'This build has no update channel configured.',
	},
	updateFailed: {
		id: 'app.settings.about.update-failed',
		defaultMessage: 'Could not reach the update server. Try again later.',
	},
	installNow: {
		id: 'app.settings.about.install-now',
		defaultMessage: 'Update now',
	},
	installing: {
		id: 'app.settings.about.installing',
		defaultMessage: 'Downloading…',
	},
	installFailed: {
		id: 'app.settings.about.install-failed',
		defaultMessage: 'The update could not be installed.',
	},
	restartHint: {
		id: 'app.settings.about.restart-hint',
		defaultMessage: 'Update installed — ACL will restart to apply it.',
	},
	contributorsTitle: {
		id: 'app.settings.about.contributors-title',
		defaultMessage: 'Contributors',
	},
	contributorAnjiuRole: {
		id: 'app.settings.about.contributor-anjiu-role',
		defaultMessage: 'Founder, product and design',
	},
	contributorAiRole: {
		id: 'app.settings.about.contributor-ai-role',
		defaultMessage: 'AI model — launcher rewrite and features',
	},
	maintainerTitle: {
		id: 'app.settings.about.maintainer-title',
		defaultMessage: 'Maintainer',
	},
	creditsTitle: {
		id: 'app.settings.about.credits-title',
		defaultMessage: 'Credits',
	},
	creditsBody: {
		id: 'app.settings.about.credits-body',
		defaultMessage:
			'Built on the open-source Modrinth launcher core and Tauri, together with every upstream dependency and its maintainers.',
	},
	brandTitle: {
		id: 'app.settings.about.brand-title',
		defaultMessage: 'Brand',
	},
	brandBody: {
		id: 'app.settings.about.brand-body',
		defaultMessage: 'Yingzhou Guild · ANJIU · © 2026',
	},
})

const version = ref('')
const updatesEnabled = ref(true)
const checking = ref(false)
const installing = ref(false)
const progress = ref(0)
const available = ref<Update | null>(null)
/** 检查结果的一句话提示；null 表示还没点过 */
const status = ref<string | null>(null)
const statusKind = ref<'info' | 'error'>('info')

onMounted(async () => {
	version.value = await getVersion().catch(() => '')
	updatesEnabled.value = await invoke<boolean>('are_updates_enabled').catch(() => false)
})

function setStatus(message: string, kind: 'info' | 'error' = 'info') {
	status.value = message
	statusKind.value = kind
}

async function checkNow() {
	if (!updatesEnabled.value) {
		setStatus(formatMessage(messages.updateDisabled), 'error')
		return
	}

	checking.value = true
	status.value = null
	available.value = null
	try {
		const update = await check()
		if (!update) {
			setStatus(formatMessage(messages.upToDate))
			return
		}
		available.value = update
		setStatus(
			formatMessage(messages.updateFound, {
				version: update.version,
				date: update.date ? new Date(update.date).toLocaleDateString() : '—',
			}),
		)
	} catch {
		setStatus(formatMessage(messages.updateFailed), 'error')
	} finally {
		checking.value = false
	}
}

async function installNow() {
	const update = available.value
	if (!update) return

	installing.value = true
	progress.value = 0
	try {
		let total = 0
		let downloaded = 0
		await update.downloadAndInstall((event) => {
			if (event.event === 'Started') {
				total = event.data.contentLength ?? 0
			} else if (event.event === 'Progress') {
				downloaded += event.data.chunkLength
				progress.value = total > 0 ? Math.min(100, (downloaded / total) * 100) : 0
			} else if (event.event === 'Finished') {
				progress.value = 100
			}
		})
		setStatus(formatMessage(messages.restartHint))
		// 安装完成后重启，让新版本接管
		await invoke('restart_app')
	} catch {
		setStatus(formatMessage(messages.installFailed), 'error')
	} finally {
		installing.value = false
	}
}
</script>

<template>
	<div class="flex flex-col gap-6">
		<div class="flex flex-col gap-1">
			<h2 class="m-0 text-lg font-semibold text-contrast">
				{{ formatMessage(messages.appName) }}
			</h2>
			<p class="m-0 text-sm text-secondary">{{ formatMessage(messages.appSubtitle) }}</p>
		</div>

		<div class="flex flex-col gap-2.5">
			<h3 class="m-0 text-base font-semibold text-contrast">
				{{ formatMessage(messages.versionTitle) }}
			</h3>
			<div class="flex flex-wrap items-center gap-3">
				<span class="rounded-lg bg-bg-raised px-3 py-1.5 font-mono text-sm text-contrast">
					v{{ version || '—' }}
				</span>
				<Button :disabled="checking || installing" @click="checkNow">
					{{ checking ? formatMessage(messages.checking) : formatMessage(messages.checkUpdate) }}
				</Button>
			</div>

			<p
				v-if="status"
				class="m-0 text-sm"
				:class="statusKind === 'error' ? 'text-red' : 'text-secondary'"
			>
				{{ status }}
			</p>

			<div v-if="available" class="flex flex-col gap-2">
				<Button :disabled="installing" @click="installNow">
					{{ installing ? formatMessage(messages.installing) : formatMessage(messages.installNow) }}
				</Button>
				<ProgressBar v-if="installing" :progress="progress" />
			</div>
		</div>

		<div class="flex flex-col gap-2.5">
			<h3 class="m-0 text-base font-semibold text-contrast">
				{{ formatMessage(messages.contributorsTitle) }}
			</h3>
			<ul class="m-0 flex list-none flex-col gap-1 p-0 text-sm">
				<li class="text-primary">
					安九
					<span class="text-secondary">— {{ formatMessage(messages.contributorAnjiuRole) }}</span>
				</li>
				<li class="text-primary">
					大肥鱼（AI 模型 · DeepSeek-V4.1-Flash）
					<span class="text-secondary">— {{ formatMessage(messages.contributorAiRole) }}</span>
				</li>
			</ul>
		</div>

		<div class="flex flex-col gap-2.5">
			<h3 class="m-0 text-base font-semibold text-contrast">
				{{ formatMessage(messages.maintainerTitle) }}
			</h3>
			<p class="m-0 text-sm text-primary">安九</p>
		</div>

		<div class="flex flex-col gap-2.5">
			<h3 class="m-0 text-base font-semibold text-contrast">
				{{ formatMessage(messages.creditsTitle) }}
			</h3>
			<p class="m-0 text-sm text-secondary">{{ formatMessage(messages.creditsBody) }}</p>
		</div>

		<div class="flex flex-col gap-2.5">
			<h3 class="m-0 text-base font-semibold text-contrast">
				{{ formatMessage(messages.brandTitle) }}
			</h3>
			<p class="m-0 text-sm text-secondary">{{ formatMessage(messages.brandBody) }}</p>
		</div>
	</div>
</template>
