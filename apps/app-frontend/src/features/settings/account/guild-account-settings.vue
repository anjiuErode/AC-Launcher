<script setup lang="ts">
import { Button, defineMessages, useVIntl } from '@modrinth/ui'
import { onMounted, ref } from 'vue'

import { describeGuildError, useGuildSession } from '@/composables/use-guild-session'

const { formatMessage } = useVIntl()

const messages = defineMessages({
	title: {
		id: 'app.guild-account.title',
		defaultMessage: 'Guild account',
	},
	description: {
		id: 'app.guild-account.description',
		defaultMessage:
			'Signs you in to guild-only content: announcements, modpacks and the member list. Uses your account on yingzhou1110.top, and is separate from your Modrinth account.',
	},
	signedInAs: {
		id: 'app.guild-account.signed-in-as',
		defaultMessage: 'Signed in as',
	},
	signOut: {
		id: 'app.guild-account.sign-out',
		defaultMessage: 'Sign out',
	},
	signIn: {
		id: 'app.guild-account.sign-in',
		defaultMessage: 'Sign in on the website',
	},
	signInHint: {
		id: 'app.guild-account.sign-in-hint',
		defaultMessage:
			'A browser window opens on yingzhou1110.top. Finish signing in there and return to ACL.',
	},
	notSignedIn: {
		id: 'app.guild-account.not-signed-in',
		defaultMessage: 'Not signed in',
	},
	waiting: {
		id: 'app.guild-account.waiting',
		defaultMessage: 'Waiting for the browser…',
	},
	signOutFailed: {
		id: 'app.guild-account.sign-out-failed',
		defaultMessage: 'Failed to sign out, please try again',
	},
})

// 与侧边栏、公会页共用同一份会话状态
const {
	session,
	busy,
	loaded,
	signInError,
	refresh,
	signIn: signInFlow,
	signOut: signOutFlow,
} = useGuildSession()

const error = ref<string | null>(null)

onMounted(async () => {
	await refresh()
	error.value = signInError.value
})

async function signIn() {
	error.value = null
	if (await signInFlow()) return
	error.value = signInError.value
}

async function signOut() {
	error.value = null
	try {
		await signOutFlow()
	} catch (err) {
		error.value = describeGuildError(err, formatMessage(messages.signOutFailed))
	}
}
</script>

<template>
	<div class="flex flex-col gap-4">
		<div class="flex flex-col gap-2">
			<span class="text-lg font-semibold text-contrast">{{ formatMessage(messages.title) }}</span>
			<span class="text-secondary">{{ formatMessage(messages.description) }}</span>
		</div>

		<div v-if="session" class="flex items-center justify-between gap-4 rounded-2xl bg-bg p-4">
			<span class="flex flex-col">
				<span class="text-sm text-secondary">{{ formatMessage(messages.signedInAs) }}</span>
				<span class="font-semibold text-contrast">{{ session.name ?? session.user_id }}</span>
			</span>
			<Button :disabled="busy" @click="signOut">{{ formatMessage(messages.signOut) }}</Button>
		</div>

		<div v-else class="flex flex-col gap-3 rounded-2xl bg-bg p-4">
			<span class="text-sm text-secondary">{{ formatMessage(messages.notSignedIn) }}</span>
			<div class="flex items-center gap-3">
				<Button :disabled="busy || !loaded" @click="signIn">
					{{ busy ? formatMessage(messages.waiting) : formatMessage(messages.signIn) }}
				</Button>
			</div>
			<span class="text-sm text-secondary">{{ formatMessage(messages.signInHint) }}</span>
		</div>

		<p v-if="error" class="m-0 text-sm text-red">{{ error }}</p>
	</div>
</template>
