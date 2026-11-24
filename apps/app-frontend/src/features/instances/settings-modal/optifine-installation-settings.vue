<script setup lang="ts">
import { ExternalIcon, PlusIcon } from '@modrinth/assets'
import { Button, defineMessages, injectNotificationManager, useVIntl } from '@modrinth/ui'
import { invoke } from '@tauri-apps/api/core'
import { openUrl } from '@tauri-apps/plugin-opener'
import { useQuery, useQueryClient } from '@tanstack/vue-query'
import { computed, ref } from 'vue'

import { toError } from '@/helpers/errors'

import { injectInstanceSettings } from './instance-settings-context'

defineProps<{ busy: boolean }>()

type OptiFineStatus =
	| 'available'
	| 'installed'
	| 'disabled'
	| 'conflict'
	| 'repair_needed'
	| 'unavailable'
	| 'unsupported'

interface OptiFineState {
	status: OptiFineStatus
	release: string | null
}

const messages = defineMessages({
	available: {
		id: 'app.optifine.available',
		defaultMessage:
			'Download OptiFine {release} from the official site, then select its JAR to install it in this instance.',
	},
	installed: { id: 'app.optifine.installed', defaultMessage: 'OptiFine {release} is installed.' },
	disabled: {
		id: 'app.optifine.disabled',
		defaultMessage: 'OptiFine is disabled. Enable it from the Mods tab.',
	},
	conflict: {
		id: 'app.optifine.conflict',
		defaultMessage:
			'This instance already contains another OptiFine file. Remove it from the Mods tab before installing.',
	},
	repair_needed: {
		id: 'app.optifine.repair-needed',
		defaultMessage: 'The OptiFine launch files are incomplete.',
	},
	unavailable: {
		id: 'app.optifine.unavailable',
		defaultMessage:
			'OptiFine can be installed in an unlocked local instance after Minecraft finishes installing.',
	},
	unsupported: {
		id: 'app.optifine.unsupported',
		defaultMessage:
			'Supports Vanilla 1.20.1 and 1.21.1, or Forge 47.2.18 on 1.20.1 and Forge 52.0.16 on 1.21.1.',
	},
	official: { id: 'app.optifine.official', defaultMessage: 'Open official site' },
	install: { id: 'app.optifine.install', defaultMessage: 'Select JAR and install' },
	statusError: { id: 'app.optifine.status-error', defaultMessage: 'Could not check OptiFine.' },
})

const { instance } = injectInstanceSettings()
const { formatMessage } = useVIntl()
const { handleError } = injectNotificationManager()
const queryClient = useQueryClient()
const installing = ref(false)
const queryKey = computed(() => [
	'instance-optifine',
	instance.value.id,
	instance.value.game_version,
	instance.value.loader,
	instance.value.loader_version,
	instance.value.install_stage,
])
const visible = computed(
	() =>
		!instance.value.link &&
		(instance.value.loader === 'vanilla' || instance.value.loader === 'forge'),
)
const statusQuery = useQuery({
	queryKey,
	queryFn: ({ queryKey }) =>
		invoke<OptiFineState>('plugin:instance|instance_optifine_status', {
			instanceId: queryKey[1],
		}),
	enabled: visible,
})
const state = statusQuery.data

async function install() {
	const key = queryKey.value
	installing.value = true
	try {
		const result = await invoke<OptiFineState | null>('plugin:instance|instance_install_optifine', {
			instanceId: key[1],
		})
		if (result) queryClient.setQueryData(key, result)
	} catch (error) {
		handleError(toError(error))
	} finally {
		installing.value = false
	}
}

async function officialSite() {
	try {
		await openUrl('https://optifine.net/downloads')
	} catch (error) {
		handleError(toError(error))
	}
}
</script>

<template>
	<section v-if="visible" class="flex flex-col gap-2.5">
		<h3 class="m-0 text-lg font-semibold text-contrast">OptiFine</h3>
		<p v-if="state" class="m-0 text-primary" role="status">
			{{ formatMessage(messages[state.status], { release: state.release ?? '' }) }}
		</p>
		<p v-else-if="statusQuery.isError.value" class="m-0 text-red" role="alert">
			{{ formatMessage(messages.statusError) }}
		</p>
		<div v-if="state?.status === 'available'" class="flex flex-wrap gap-2">
			<Button type="outlined" :disabled="installing" @click="officialSite">
				<ExternalIcon />
				{{ formatMessage(messages.official) }}
			</Button>
			<Button type="colored" color="brand" :disabled="busy" :loading="installing" @click="install">
				<PlusIcon />
				{{ formatMessage(messages.install) }}
			</Button>
		</div>
	</section>
</template>
