<script setup lang="ts">
import { Button, defineMessages, useVIntl } from '@modrinth/ui'
import { invoke } from '@tauri-apps/api/core'
import { computed, onMounted, ref, watch } from 'vue'

type ProxyMode = 'auto' | 'off' | 'manual'

type ProxyConfig = {
	mode: ProxyMode
	url: string | null
}

interface ProxySettingsSnapshot {
	saved: ProxyConfig
	active_mode: ProxyMode
	saved_valid: boolean
	pending_restart: boolean
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
	urlLabel: {
		id: 'app.network-settings.proxy.url-label',
		defaultMessage: 'Proxy address',
	},
	test: {
		id: 'app.network-settings.proxy.test',
		defaultMessage: 'Test connection',
	},
	testing: {
		id: 'app.network-settings.proxy.testing',
		defaultMessage: 'Testing…',
	},
	testFailed: {
		id: 'app.network-settings.proxy.test-failed',
		defaultMessage: 'Could not start the connection test. Try again.',
	},
	testHint: {
		id: 'app.network-settings.proxy.test-hint',
		defaultMessage:
			'Checks Modrinth and Microsoft sign-in with the settings above, without saving them.',
	},
	save: {
		id: 'app.network-settings.proxy.save',
		defaultMessage: 'Save',
	},
	saving: {
		id: 'app.network-settings.proxy.saving',
		defaultMessage: 'Saving…',
	},
	saveFailed: {
		id: 'app.network-settings.proxy.save-failed',
		defaultMessage: 'Could not save the proxy settings. Try again.',
	},
	saved: {
		id: 'app.network-settings.proxy.saved',
		defaultMessage: 'Proxy settings saved.',
	},
	loading: {
		id: 'app.network-settings.proxy.loading',
		defaultMessage: 'Loading proxy settings…',
	},
	loadFailed: {
		id: 'app.network-settings.proxy.load-failed',
		defaultMessage: 'Could not read the saved proxy settings.',
	},
	retry: {
		id: 'app.network-settings.proxy.retry',
		defaultMessage: 'Try again',
	},
	current: {
		id: 'app.network-settings.proxy.current',
		defaultMessage: 'Currently in effect',
	},
	pendingRestart: {
		id: 'app.network-settings.proxy.pending-restart',
		defaultMessage: 'Restart ACL to apply the saved proxy settings.',
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
		defaultMessage: 'Manual proxy',
	},
	savedInvalid: {
		id: 'app.network-settings.proxy.saved-invalid',
		defaultMessage:
			'The saved proxy address is invalid. ACL falls back to a direct connection when started with this setting.',
	},
	invalidAddress: {
		id: 'app.network-settings.proxy.invalid-address',
		defaultMessage: 'Enter a valid HTTP or HTTPS proxy address.',
	},
	reachable: {
		id: 'app.network-settings.proxy.reachable',
		defaultMessage: 'Reachable',
	},
	unreachable: {
		id: 'app.network-settings.proxy.unreachable',
		defaultMessage: 'Failed',
	},
	probeTimeout: {
		id: 'app.network-settings.proxy.probe-timeout',
		defaultMessage: 'Connection timed out',
	},
	probeError: {
		id: 'app.network-settings.proxy.probe-error',
		defaultMessage: 'Connection request failed',
	},
})

const mode = ref<ProxyMode>('auto')
const url = ref('')
const snapshot = ref<ProxySettingsSnapshot | null>(null)
const loaded = computed(() => snapshot.value !== null)
const loading = ref(false)
const loadError = ref(false)
const testing = ref(false)
const saving = ref(false)
const probes = ref<NetProbe[]>([])
const testError = ref<string | null>(null)
const saveError = ref<string | null>(null)
const saveMessage = ref<string | null>(null)
const savedConfig = computed(() => snapshot.value?.saved ?? null)
const pendingRestart = computed(() => snapshot.value?.pending_restart ?? false)
const savedInvalid = computed(() => snapshot.value?.saved_valid === false)
const manualAddress = computed(() => (mode.value === 'manual' ? url.value.trim() : null))
let draftRevision = 0

const currentLabel = computed(() => {
	const activeMode = snapshot.value?.active_mode
	if (activeMode === 'manual') return formatMessage(messages.currentManual)
	if (activeMode === 'off') return formatMessage(messages.currentOff)
	return activeMode === 'auto' ? formatMessage(messages.currentAuto) : ''
})

const hasUnsavedChanges = computed(() => {
	const config = savedConfig.value
	if (!config) return false
	const { mode: nextMode, url: nextUrl } = payload()
	return config.mode !== nextMode || (config.url?.trim() ?? '') !== (nextUrl ?? '')
})

function payload(): ProxyConfig {
	return { mode: mode.value, url: manualAddress.value }
}

watch(
	[mode, manualAddress],
	() => {
		draftRevision++
		probes.value = []
		testError.value = null
		saveError.value = null
		saveMessage.value = null
	},
	{ flush: 'sync' },
)

async function load() {
	if (loaded.value || loading.value) return
	loading.value = true
	loadError.value = false
	try {
		const config = await invoke<ProxySettingsSnapshot>('get_proxy_config')
		mode.value = config.saved.mode
		url.value = config.saved.url ?? ''
		snapshot.value = config
	} catch {
		loadError.value = true
	} finally {
		loading.value = false
	}
}

onMounted(load)

async function runTest() {
	if (!loaded.value || testing.value) return
	const revision = draftRevision
	const config = payload()
	testing.value = true
	probes.value = []
	testError.value = null
	try {
		const result = await invoke<NetProbe[]>('test_network', config)
		if (draftRevision === revision) probes.value = result
	} catch (error) {
		if (draftRevision === revision) {
			testError.value = formatMessage(
				error === 'invalid_address' ? messages.invalidAddress : messages.testFailed,
			)
		}
	} finally {
		testing.value = false
	}
}

async function save() {
	if (!loaded.value || saving.value || !hasUnsavedChanges.value) return
	const revision = draftRevision
	const config = payload()
	saving.value = true
	saveError.value = null
	saveMessage.value = null
	try {
		snapshot.value = await invoke<ProxySettingsSnapshot>('set_proxy_config', config)
		saveMessage.value = formatMessage(messages.saved)
	} catch (error) {
		saveError.value = formatMessage(
			draftRevision === revision && error === 'invalid_address'
				? messages.invalidAddress
				: messages.saveFailed,
		)
	} finally {
		saving.value = false
	}
}
</script>

<template>
	<div class="flex flex-col gap-4">
		<div class="flex flex-col gap-2">
			<span class="text-lg font-semibold text-contrast">{{ formatMessage(messages.title) }}</span>
			<span class="text-secondary">{{ formatMessage(messages.description) }}</span>
		</div>

		<div v-if="loaded" class="flex flex-col gap-1 rounded-2xl bg-bg-raised p-4" aria-live="polite">
			<span class="text-sm text-secondary">{{ formatMessage(messages.current) }}</span>
			<span class="font-semibold text-contrast">{{ currentLabel }}</span>
			<span v-if="hasUnsavedChanges" class="text-sm text-brand">
				{{ formatMessage(messages.unsavedChanges) }}
			</span>
			<span v-if="pendingRestart" class="text-sm text-secondary">
				{{ formatMessage(messages.pendingRestart) }}
			</span>
			<span v-if="savedInvalid" class="text-sm text-red">
				{{ formatMessage(messages.savedInvalid) }}
			</span>
		</div>
		<p v-else-if="loading" class="m-0 text-sm text-secondary" role="status">
			{{ formatMessage(messages.loading) }}
		</p>
		<div v-else-if="loadError" class="flex flex-wrap items-center gap-2">
			<p class="m-0 text-sm text-red" role="alert">{{ formatMessage(messages.loadFailed) }}</p>
			<Button @click="load">{{ formatMessage(messages.retry) }}</Button>
		</div>

		<fieldset :disabled="!loaded" class="m-0 flex min-w-0 flex-col gap-2 border-none p-0">
			<legend class="sr-only">{{ formatMessage(messages.title) }}</legend>
			<label class="flex cursor-pointer items-start gap-3 rounded-2xl bg-bg p-4">
				<input v-model="mode" type="radio" name="network-proxy-mode" value="auto" class="mt-1" />
				<span class="flex flex-col">
					<span class="font-semibold text-contrast">{{ formatMessage(messages.modeAuto) }}</span>
					<span class="text-sm text-secondary">{{ formatMessage(messages.modeAutoDesc) }}</span>
				</span>
			</label>

			<label class="flex cursor-pointer items-start gap-3 rounded-2xl bg-bg p-4">
				<input v-model="mode" type="radio" name="network-proxy-mode" value="off" class="mt-1" />
				<span class="flex flex-col">
					<span class="font-semibold text-contrast">{{ formatMessage(messages.modeOff) }}</span>
					<span class="text-sm text-secondary">{{ formatMessage(messages.modeOffDesc) }}</span>
				</span>
			</label>

			<label class="flex cursor-pointer items-start gap-3 rounded-2xl bg-bg p-4">
				<input v-model="mode" type="radio" name="network-proxy-mode" value="manual" class="mt-1" />
				<span class="flex flex-col">
					<span class="font-semibold text-contrast">{{ formatMessage(messages.modeManual) }}</span>
					<span class="text-sm text-secondary">{{ formatMessage(messages.modeManualDesc) }}</span>
				</span>
			</label>

			<input
				v-if="mode === 'manual'"
				v-model="url"
				type="text"
				autocomplete="off"
				spellcheck="false"
				:aria-label="formatMessage(messages.urlLabel)"
				:placeholder="formatMessage(messages.urlPlaceholder)"
				class="w-full rounded-xl bg-bg px-4 py-3 text-contrast outline-none"
			/>
		</fieldset>

		<div class="flex items-center gap-2">
			<Button :disabled="!loaded" :loading="testing" @click="runTest">
				{{ formatMessage(testing ? messages.testing : messages.test) }}
			</Button>
			<Button :disabled="!loaded || !hasUnsavedChanges" :loading="saving" @click="save">
				{{ formatMessage(saving ? messages.saving : messages.save) }}
			</Button>
		</div>

		<p v-if="saveError" class="m-0 text-sm text-red" role="alert">{{ saveError }}</p>
		<p v-if="saveMessage" class="m-0 text-sm text-secondary" role="status">{{ saveMessage }}</p>
		<p class="m-0 text-sm text-secondary">{{ formatMessage(messages.testHint) }}</p>
		<p v-if="testError" class="m-0 text-sm text-red" role="alert">{{ testError }}</p>

		<div v-if="probes.length" class="flex flex-col gap-2" aria-live="polite">
			<div
				v-for="probe in probes"
				:key="probe.url"
				class="flex items-center justify-between gap-4 rounded-xl bg-bg p-3"
			>
				<span class="flex flex-col">
					<span class="font-medium text-contrast">{{ probe.label }}</span>
					<span class="text-xs text-secondary">{{ probe.url }}</span>
					<span v-if="probe.error" class="text-xs text-red">
						{{
							formatMessage(probe.error === 'timeout' ? messages.probeTimeout : messages.probeError)
						}}
					</span>
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
	</div>
</template>
