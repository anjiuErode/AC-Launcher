<script setup lang="ts">
import { FolderArchiveIcon, SpinnerIcon } from '@modrinth/assets'
import { Button, defineMessages, injectNotificationManager, useVIntl } from '@modrinth/ui'
import { useQueryClient } from '@tanstack/vue-query'
import { open } from '@tauri-apps/plugin-dialog'
import { ref } from 'vue'
import { useRouter } from 'vue-router'

import { restoreInstanceArchive } from '@/helpers/instance'
import { instanceKeys } from '@/pages/instance/query-options'

const props = defineProps<{ size?: 'lg' }>()
const messages = defineMessages({
	restore: {
		id: 'app.instance.archive.restore',
		defaultMessage: 'Restore instance archive',
	},
	restoring: {
		id: 'app.instance.archive.restoring',
		defaultMessage: 'Restoring instance…',
	},
	restored: {
		id: 'app.instance.archive.restored',
		defaultMessage: 'Instance restored as a new copy',
	},
})
const { formatMessage } = useVIntl()
const { addNotification, handleError } = injectNotificationManager()
const queryClient = useQueryClient()
const router = useRouter()
const restoring = ref(false)

async function restore() {
	if (restoring.value) return
	restoring.value = true
	try {
		const archive = await open({ filters: [{ name: 'ACL ZIP', extensions: ['zip'] }] })
		if (!archive) return
		const id = await restoreInstanceArchive(archive)
		await queryClient.invalidateQueries({ queryKey: instanceKeys.list() })
		addNotification({ type: 'success', title: formatMessage(messages.restored) })
		await router.push(`/instance/${encodeURIComponent(id)}`)
	} catch (error) {
		handleError(error)
	} finally {
		restoring.value = false
	}
}
</script>

<template>
	<Button :size="props.size" :disabled="restoring" @click="restore">
		<SpinnerIcon v-if="restoring" class="animate-spin" />
		<FolderArchiveIcon v-else />
		{{ formatMessage(restoring ? messages.restoring : messages.restore) }}
	</Button>
</template>
