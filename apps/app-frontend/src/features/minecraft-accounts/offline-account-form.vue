<script setup lang="ts">
import { Button, defineMessages, injectNotificationManager, useVIntl } from '@modrinth/ui'
import { computed, ref } from 'vue'

import { create_offline_user } from '@/helpers/auth'

const emit = defineEmits<{ created: []; cancel: [] }>()
const { formatMessage } = useVIntl()
const { handleError } = injectNotificationManager()
const name = ref('')
const saving = ref(false)
const validName = computed(() => /^[A-Za-z0-9_]{3,16}$/.test(name.value))

defineExpose({ busy: saving })

const messages = defineMessages({
	name: { id: 'app.offline-account.name', defaultMessage: 'Player name' },
	create: { id: 'app.offline-account.create', defaultMessage: 'Create' },
	cancel: { id: 'app.offline-account.cancel', defaultMessage: 'Cancel' },
})

async function addOfflineAccount() {
	if (saving.value || !validName.value) return
	saving.value = true
	try {
		await create_offline_user(name.value)
		emit('created')
	} catch (error) {
		handleError(error)
	} finally {
		saving.value = false
	}
}
</script>

<template>
	<form class="flex flex-col gap-2" @submit.prevent="addOfflineAccount">
		<label class="flex flex-col gap-2 text-sm">
			{{ formatMessage(messages.name) }}
			<input
				v-model.trim="name"
				type="text"
				autocomplete="off"
				minlength="3"
				maxlength="16"
				pattern="[A-Za-z0-9_]{3,16}"
				required
				class="w-full"
			/>
		</label>
		<div class="flex flex-wrap gap-2">
			<Button type="colored" color="brand" native-type="submit" :disabled="saving || !validName">
				{{ formatMessage(messages.create) }}
			</Button>
			<Button :disabled="saving" @click="emit('cancel')">
				{{ formatMessage(messages.cancel) }}
			</Button>
		</div>
	</form>
</template>
