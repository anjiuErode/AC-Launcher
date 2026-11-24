<script setup lang="ts">
import { Button, defineMessages, injectNotificationManager, useVIntl } from '@modrinth/ui'
import { computed, ref, watch } from 'vue'
import { useRouter } from 'vue-router'

import { useAppEvent } from '@/composables/use-app-event'
import { install_existing_instance, wait_for_install_job } from '@/helpers/install'
import { get_launch_diagnostic } from '@/helpers/logs'
import { injectAppEvents } from '@/providers/app-events'

import { injectInstancePage } from '../instance-context'

type LaunchIssue =
	| { kind: 'java' | 'game_library' | 'unknown_exit' }
	| { kind: 'missing_mod'; mod_id: string }

interface LaunchDiagnostic {
	run_id: string
	exit_code: number | null
	issue: LaunchIssue
}

const props = defineProps<{ launchFailureCount: number }>()
const emit = defineEmits<{ openJava: [] }>()
const { formatMessage } = useVIntl()
const { handleError } = injectNotificationManager()
const events = injectAppEvents()
const router = useRouter()
const page = injectInstancePage()
const diagnostic = ref<LaunchDiagnostic | null>(null)
const dismissedRun = ref<string | null>(null)
const repairing = ref(false)
let requestId = 0

const messages = defineMessages({
	java: {
		id: 'app.launch-diagnostic.java',
		defaultMessage:
			'The Java runtime could not start this instance. Check the Java version in the instance settings.',
	},
	missing_mod: {
		id: 'app.launch-diagnostic.missing-mod',
		defaultMessage:
			'The loader reports a missing mod ({mod}). Review the compatible versions before installing it.',
	},
	game_library: {
		id: 'app.launch-diagnostic.game-library',
		defaultMessage:
			'A game library appears missing or damaged. Repair the game files before launching again.',
	},
	unknown_exit: {
		id: 'app.launch-diagnostic.unknown-exit',
		defaultMessage:
			'Minecraft exited unexpectedly. Review this instance’s game log for more information.',
	},
	title: { id: 'app.launch-diagnostic.title', defaultMessage: 'Launch diagnosis' },
	javaSettings: { id: 'app.launch-diagnostic.java-settings', defaultMessage: 'Open Java settings' },
	findMod: { id: 'app.launch-diagnostic.find-mod', defaultMessage: 'Find dependency' },
	repair: { id: 'app.launch-diagnostic.repair', defaultMessage: 'Repair and relaunch' },
	logs: { id: 'app.launch-diagnostic.logs', defaultMessage: 'View game logs' },
	dismiss: { id: 'app.launch-diagnostic.dismiss', defaultMessage: 'Dismiss' },
})

const visibleIssue = computed(() =>
	!page.playing.value &&
	!page.instance.value.quarantined &&
	diagnostic.value?.run_id !== dismissedRun.value
		? diagnostic.value?.issue
		: undefined,
)
const description = computed(() => {
	const issue = visibleIssue.value
	if (!issue) return ''
	return formatMessage(
		messages[issue.kind],
		issue.kind === 'missing_mod' ? { mod: issue.mod_id } : {},
	)
})

async function refreshDiagnostic(runId?: string) {
	const instanceId = page.instanceId.value
	const request = ++requestId
	const value = (await get_launch_diagnostic(instanceId).catch(
		() => null,
	)) as LaunchDiagnostic | null
	if (
		request === requestId &&
		instanceId === page.instanceId.value &&
		(!runId || !value || runId === value.run_id)
	) {
		diagnostic.value = value
	}
}

watch([page.instanceId, () => props.launchFailureCount], () => void refreshDiagnostic(), {
	immediate: true,
})
useAppEvent('process', (event) => {
	if (event.instance_id !== page.instanceId.value) return
	if (event.event === 'launched') {
		requestId++
		diagnostic.value = null
	} else {
		void refreshDiagnostic(event.uuid)
	}
})

async function findDependency() {
	const issue = visibleIssue.value
	if (issue?.kind !== 'missing_mod') return
	await router.push({
		path: '/browse/mod',
		query: { i: page.instanceId.value, q: issue.mod_id },
	})
}

async function repairAndRelaunch() {
	if (repairing.value) return
	const instanceId = page.instanceId.value
	repairing.value = true
	try {
		const job = await install_existing_instance(instanceId, true)
		await wait_for_install_job(events, job.job_id)
		if (page.instanceId.value === instanceId) await page.play('LaunchDiagnostic')
	} catch (error) {
		handleError(error)
	} finally {
		repairing.value = false
	}
}
</script>

<template>
	<section
		v-if="visibleIssue"
		class="mt-4 rounded-xl border border-solid border-surface-5 bg-surface-2 p-4"
	>
		<h2 class="m-0 text-base font-semibold">{{ formatMessage(messages.title) }}</h2>
		<p class="my-2 text-secondary">{{ description }}</p>
		<div class="flex flex-wrap gap-2">
			<Button v-if="visibleIssue.kind === 'java'" @click="emit('openJava')">
				{{ formatMessage(messages.javaSettings) }}
			</Button>
			<Button
				v-if="visibleIssue.kind === 'missing_mod'"
				:disabled="page.offline.value"
				@click="findDependency"
			>
				{{ formatMessage(messages.findMod) }}
			</Button>
			<Button
				v-if="visibleIssue.kind === 'game_library'"
				:disabled="repairing || page.offline.value"
				@click="repairAndRelaunch"
			>
				{{ formatMessage(messages.repair) }}
			</Button>
			<Button @click="router.push({ name: 'InstanceLogs', params: { id: page.instanceId.value } })">
				{{ formatMessage(messages.logs) }}
			</Button>
			<Button @click="dismissedRun = diagnostic?.run_id ?? null">
				{{ formatMessage(messages.dismiss) }}
			</Button>
		</div>
	</section>
</template>
