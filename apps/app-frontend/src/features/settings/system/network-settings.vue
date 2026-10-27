<script setup lang="ts">
import { Button, defineMessages, useVIntl } from '@modrinth/ui'
import { invoke } from '@tauri-apps/api/core'
import { computed, onMounted, ref } from 'vue'

type ProxyMode = 'auto' | 'off' | 'manual'

interface ProxyConfig {
	mode: ProxyMode
	url: string | null
}

interface NetProbe {
	label: string
	url: string
	ok: boolean
	status: number | null
	millis: number
	error: string | null
}

const { formatMessage } = useVIntl()

const messages = defineMessages({
	title: {
		id: 'app.network-settings.proxy.title',
		defaultMessage: 'Network proxy',
	},
	description: {
		id: 'app.network-settings.proxy.description',
		defaultMessage:
			'Controls how ACL reaches Modrinth and Microsoft sign-in. Keep it on automatic unless you know you need otherwise.',
	},
	modeAuto: {
		id: 'app.network-settings.proxy.mode-auto',
		defaultMessage: 'Automatic',
	},
	modeAutoDesc: {
		id: 'app.network-settings.proxy.mode-auto-desc',
		defaultMessage: 'Follow the system proxy, then the proxy environment variables',
	},
	modeOff: {
		id: 'app.network-settings.proxy.mode-off',
		defaultMessage: 'Direct connection',
	},
	modeOffDesc: {
		id: 'app.network-settings.proxy.mode-off-desc',
		defaultMessage: 'Ignore every proxy setting and connect directly',
	},
	modeManual: {
		id: 'app.network-settings.proxy.mode-manual',
		defaultMessage: 'Manual',
	},
	modeManualDesc: {
		id: 'app.network-settings.proxy.mode-manual-desc',
		defaultMessage: 'Use the proxy address below',
	},
	urlPlaceholder: {
		id: 'app.network-settings.proxy.url-placeholder',
		defaultMessage: 'http://127.0.0.1:7890',
	},
	test: {
		id: 'app.network-settings.proxy.test',
		defaultMessage: 'Test connection',
	},
	testHint: {
		id: 'app.network-settings.proxy.test-hint',
		defaultMessage: 'Checks Modrinth and Microsoft sign-in with the settings above, without saving them.',
	},
	save: {
		id: 'app.network-settings.proxy.save',
		defaultMessage: 'Save',
	},
	saved: {
		id: 'app.network-settings.proxy.saved',
		defaultMessage: 'Saved. Restart ACL for the new proxy setting to take effect.',
	},
	current: {
		id: 'app.network-settings.proxy.current',
		defaultMessage: 'Currently in effect',
	},
	pendingRestart: {
		id: 'app.network-settings.proxy.pending-restart',
		defaultMessage: 'Changed settings take effect after restarting ACL.',
	},
	unsavedChanges: {
		id: 'app.network-settings.proxy.unsaved-changes',
		defaultMessage: 'Unsaved changes',
	},
	currentAuto: {
		id: 'app.network-settings.proxy.current-auto',
		defaultMessage: 'Automatic · follow system proxy, then environment variables',
	},
	currentOff: {
		id: 'app.network-settings.proxy.current-off',
		defaultMessage: 'Direct connection · all proxies ignored',
	},
	currentManual: {
		id: 'app.network-settings.proxy.current-manual',
		defaultMessage: 'Manual · {url}',
	},
	currentManualEmpty: {
		id: 'app.network-settings.proxy.current-manual-empty',
		defaultMessage: 'Manual · no address set',
	},
	reachable: {
		id: 'app.network-settings.proxy.reachable',
		defaultMessage: 'Reachable',
	},
	unreachable: {
		id: 'app.network-settings.proxy.unreachable',
		defaultMessage: 'Failed',
	},
})

const mode = ref<ProxyMode>('auto')
const url = ref('')
const loaded = ref(false)
const testing = ref(false)
const probes = ref<NetProbe[]>([])
const message = ref<string | null>(null)
/** 已保存到磁盘的配置；与表单分开，用来显示「当前生效」 */
const savedConfig = ref<ProxyConfig | null>(null)

const currentLabel = computed(() => {
	const config = savedConfig.value
	if (!config) return ''
	if (config.mode === 'manual') {
		return config.url
			? formatMessage(messages.currentManual, { url: config.url })
			: formatMessage(messages.currentManualEmpty)
	}
	return config.mode === 'off'
		? formatMessage(messages.currentOff)
		: formatMessage(messages.currentAuto)
})

const hasUnsavedChanges = computed(() => {
	const config = savedConfig.value
	if (!config) return false
	const { mode: nextMode, url: nextUrl } = payload()
	return config.mode !== nextMode || (config.url ?? '') !== (nextUrl ?? '')
})

function payload(): { mode: ProxyMode; url: string | null } {
	return { mode: mode.value, url: mode.value === 'manual' ? url.value.trim() : null }
}

onMounted(async () => {
	try {
		const config = await invoke<ProxyConfig>('get_proxy_config')
		mode.value = config.mode ?? 'auto'
		url.value = config.url ?? ''
		savedConfig.value = { mode: mode.value, url: config.url ?? null }
	} catch (err) {
		message.value = String(err)
	} finally {
		loaded.value = true
	}
})

async function runTest() {
	testing.value = true
	message.value = null
	try {
		probes.value = await invoke<NetProbe[]>('test_network', payload())
	} catch (err) {
		message.value = String(err)
	} finally {
		testing.value = false
	}
}

async function save() {
	try {
		const { mode: nextMode, url: nextUrl } = payload()
		await invoke('set_proxy_config', { mode: nextMode, url: nextUrl })
		savedConfig.value = { mode: nextMode, url: nextUrl }
		message.value = formatMessage(messages.saved)
	} catch (err) {
		message.value = String(err)
	}
}
</script>

<template>
	<div class="flex flex-col gap-4">
		<div class="flex flex-col gap-2">
			<span class="text-lg font-semibold text-contrast">{{ formatMessage(messages.title) }}</span>
			<span class="text-secondary">{{ formatMessage(messages.description) }}</span>
		</div>

		<div class="flex flex-col gap-1 rounded-2xl bg-bg-raised p-4">
			<span class="text-sm text-secondary">{{ formatMessage(messages.current) }}</span>
			<span class="font-semibold text-contrast">{{ currentLabel }}</span>
			<span v-if="hasUnsavedChanges" class="text-sm text-brand">
				{{ formatMessage(messages.unsavedChanges) }}
			</span>
			<span class="text-sm text-secondary">
				{{ formatMessage(messages.pendingRestart) }}
			</span>
		</div>

		<div class="flex flex-col gap-2">
			<label class="flex cursor-pointer items-start gap-3 rounded-2xl bg-bg p-4">
				<input v-model="mode" type="radio" value="auto" :disabled="!loaded" class="mt-1" />
				<span class="flex flex-col">
					<span class="font-semibold text-contrast">{{ formatMessage(messages.modeAuto) }}</span>
					<span class="text-sm text-secondary">{{ formatMessage(messages.modeAutoDesc) }}</span>
				</span>
			</label>

			<label class="flex cursor-pointer items-start gap-3 rounded-2xl bg-bg p-4">
				<input v-model="mode" type="radio" value="off" :disabled="!loaded" class="mt-1" />
				<span class="flex flex-col">
					<span class="font-semibold text-contrast">{{ formatMessage(messages.modeOff) }}</span>
					<span class="text-sm text-secondary">{{ formatMessage(messages.modeOffDesc) }}</span>
				</span>
			</label>

			<label class="flex cursor-pointer items-start gap-3 rounded-2xl bg-bg p-4">
				<input v-model="mode" type="radio" value="manual" :disabled="!loaded" class="mt-1" />
				<span class="flex flex-col">
					<span class="font-semibold text-contrast">{{ formatMessage(messages.modeManual) }}</span>
					<span class="text-sm text-secondary">{{ formatMessage(messages.modeManualDesc) }}</span>
				</span>
			</label>

			<input
				v-if="mode === 'manual'"
				v-model="url"
				type="text"
				spellcheck="false"
				:placeholder="formatMessage(messages.urlPlaceholder)"
				class="w-full rounded-xl bg-bg px-4 py-3 text-contrast outline-none"
			/>
		</div>

		<div class="flex items-center gap-2">
			<Button :disabled="testing || !loaded" @click="runTest">
				{{ testing ? '…' : formatMessage(messages.test) }}
			</Button>
			<Button :disabled="!loaded" @click="save">{{ formatMessage(messages.save) }}</Button>
		</div>

		<p class="m-0 text-sm text-secondary">{{ formatMessage(messages.testHint) }}</p>

		<div v-if="probes.length" class="flex flex-col gap-2">
			<div
				v-for="probe in probes"
				:key="probe.url"
				class="flex items-center justify-between gap-4 rounded-xl bg-bg p-3"
			>
				<span class="flex flex-col">
					<span class="font-medium text-contrast">{{ probe.label }}</span>
					<span class="text-xs text-secondary">{{ probe.url }}</span>
					<span v-if="probe.error" class="text-xs text-red">{{ probe.error }}</span>
				</span>
				<span class="flex flex-col items-end">
					<span :class="probe.ok ? 'text-green' : 'text-red'" class="font-semibold">
						{{ probe.ok ? formatMessage(messages.reachable) : formatMessage(messages.unreachable) }}
					</span>
					<span class="text-xs text-secondary">
						{{ probe.status ? `HTTP ${probe.status} · ` : '' }}{{ probe.millis }} ms
					</span>
				</span>
			</div>
		</div>

		<p v-if="message" class="m-0 text-sm text-secondary">{{ message }}</p>
	</div>
</template>
