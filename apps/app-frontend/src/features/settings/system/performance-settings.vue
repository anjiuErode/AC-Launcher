<script setup lang="ts">
import { defineMessages, Toggle, useVIntl } from '@modrinth/ui'
import { invoke } from '@tauri-apps/api/core'
import { onMounted, ref } from 'vue'

const { formatMessage } = useVIntl()

const messages = defineMessages({
	title: {
		id: 'app.performance-settings.hardware-acceleration.title',
		defaultMessage: 'Hardware acceleration',
	},
	description: {
		id: 'app.performance-settings.hardware-acceleration.description',
		defaultMessage:
			'Render the interface with the GPU. Turn this off if the window turns black, flickers, or the driver crashes.',
	},
	restartHint: {
		id: 'app.performance-settings.hardware-acceleration.restart-hint',
		defaultMessage: 'Changes take effect after restarting ACL.',
	},
})

const enabled = ref(true)
const loaded = ref(false)

onMounted(async () => {
	try {
		enabled.value = await invoke<boolean>('get_hardware_acceleration')
	} finally {
		loaded.value = true
	}
})

async function onToggle(value: boolean) {
	enabled.value = value
	await invoke('set_hardware_acceleration', { enabled: value })
}
</script>

<template>
	<div class="flex flex-col gap-3">
		<div class="flex items-center justify-between gap-6 rounded-2xl bg-bg p-4">
			<div class="flex flex-col">
				<span class="text-lg font-semibold text-contrast">{{ formatMessage(messages.title) }}</span>
				<span class="mt-1 text-secondary">{{ formatMessage(messages.description) }}</span>
			</div>
			<Toggle
				id="hardware-acceleration"
				:model-value="enabled"
				:disabled="!loaded"
				@update:model-value="onToggle"
			/>
		</div>
		<p class="m-0 text-sm text-secondary">{{ formatMessage(messages.restartHint) }}</p>
	</div>
</template>
