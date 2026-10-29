<script setup lang="ts">
import { Button, defineMessages, ProgressBar, useVIntl } from '@modrinth/ui'
import { getVersion } from '@tauri-apps/api/app'
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'

import {
	appUpdateState,
	checkForAppUpdate,
	downloadAvailableAppUpdate,
	installAvailableAppUpdate,
} from '@/providers/app-update'

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
		defaultMessage: 'Version {version} is available.',
	},
	detectTimeout: {
		id: 'app.settings.about.detect-timeout',
		defaultMessage: 'Update channel detection timed out. Restart ACL and try again.',
	},
	detecting: {
		id: 'app.settings.about.detecting',
		defaultMessage: 'Detecting the update channel…',
	},
	updateDisabled: {
		id: 'app.settings.about.update-disabled',
		defaultMessage: 'This build has no update channel configured.',
	},
	updateFailed: {
		id: 'app.settings.about.update-failed',
		defaultMessage: 'Could not reach the update server. Try again later.',
	},
	downloadUpdate: {
		id: 'app.settings.about.download-update',
		defaultMessage: 'Download update',
	},
	downloading: {
		id: 'app.settings.about.downloading',
		defaultMessage: 'Downloading…',
	},
	restartInstall: {
		id: 'app.settings.about.restart-install',
		defaultMessage: 'Restart and install',
	},
	downloadedHint: {
		id: 'app.settings.about.downloaded-hint',
		defaultMessage: 'Update downloaded. Restart to install it.',
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

// 全部来自共享更新状态：与主界面自动检查是同一个任务，不会重复下载
const updatesEnabled = computed(() => appUpdateState.updatesEnabled.value)
const capabilityResolved = computed(() => appUpdateState.updatesCapabilityResolved.value)
// 本地 IPC 正常应在毫秒级返回；超过 5 秒视为探测异常，避免永远停在"检测中"
const capabilityTimedOut = ref(false)
let capabilityTimer: ReturnType<typeof setTimeout> | null = null
const checking = computed(() => appUpdateState.checking.value)
const downloading = computed(() => appUpdateState.downloading.value)
const downloaded = computed(() => appUpdateState.finishedDownloading.value)
const progress = computed(() => appUpdateState.progress.value)
const available = computed(() => appUpdateState.availableUpdate.value)
const checkError = computed(() => appUpdateState.checkError.value)

/** 手动检查的结果文案；null 表示本次尚未检查 */
const manualResult = ref<string | null>(null)
const manualResultIsError = ref(false)

onMounted(async () => {
	version.value = await getVersion().catch(() => '')
	capabilityTimer = setTimeout(() => {
		if (!capabilityResolved.value) capabilityTimedOut.value = true
	}, 5000)
})

onBeforeUnmount(() => {
	if (capabilityTimer) clearTimeout(capabilityTimer)
})

// 探测到时提前清掉计时器，避免超时提示在已就绪后闪一下
watch(capabilityResolved, (resolved) => {
	if (resolved && capabilityTimer) {
		clearTimeout(capabilityTimer)
		capabilityTimer = null
	}
})

function setResult(message: string, isError = false) {
	manualResult.value = message
	manualResultIsError.value = isError
}

async function checkNow() {
	// 还没探测完就别让点：此时 updatesEnabled 仍是初值 false，
	// 直接提示「未接入更新通道」会误导
	if (!capabilityResolved.value) return
	if (!updatesEnabled.value) {
		setResult(formatMessage(messages.updateDisabled), true)
		return
	}

	manualResult.value = null
	try {
		const update = await checkForAppUpdate()
		if (!update) {
			setResult(formatMessage(messages.upToDate))
			return
		}
		setResult(formatMessage(messages.updateFound, { version: update.version }))
	} catch {
		setResult(formatMessage(messages.updateFailed), true)
	}
}

async function downloadNow() {
	try {
		await downloadAvailableAppUpdate()
	} catch {
		setResult(formatMessage(messages.updateFailed), true)
	}
}

async function installNow() {
	try {
		await installAvailableAppUpdate()
	} catch {
		setResult(formatMessage(messages.updateFailed), true)
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
				<Button :disabled="!capabilityResolved || checking || downloading" @click="checkNow">
					{{ checking ? formatMessage(messages.checking) : formatMessage(messages.checkUpdate) }}
				</Button>
			</div>

			<p v-if="!capabilityResolved" class="m-0 text-sm text-secondary">
				{{
					capabilityTimedOut
						? formatMessage(messages.detectTimeout)
						: formatMessage(messages.detecting)
				}}
			</p>
			<p
				v-else-if="manualResult"
				class="m-0 text-sm"
				:class="manualResultIsError ? 'text-red' : 'text-secondary'"
			>
				{{ manualResult }}
			</p>
			<p v-else-if="checkError" class="m-0 text-sm text-red">{{ checkError }}</p>

			<template v-if="available">
				<div class="flex flex-wrap items-center gap-3">
					<Button v-if="!downloaded" :disabled="downloading" @click="downloadNow">
						{{
							downloading
								? formatMessage(messages.downloading)
								: formatMessage(messages.downloadUpdate)
						}}
					</Button>
					<Button v-else @click="installNow">
						{{ formatMessage(messages.restartInstall) }}
					</Button>
					<span v-if="downloaded" class="text-sm text-secondary">
						{{ formatMessage(messages.downloadedHint) }}
					</span>
				</div>
				<ProgressBar v-if="downloading" :progress="progress" />
			</template>
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
