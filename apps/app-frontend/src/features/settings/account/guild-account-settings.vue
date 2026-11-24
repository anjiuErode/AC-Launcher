<script setup lang="ts">
import { SpinnerIcon } from '@modrinth/assets'
import { Button, defineMessages, useVIntl } from '@modrinth/ui'
import { computed, onMounted } from 'vue'

import { useGuildSession } from '@/composables/use-guild-session'

const { formatMessage } = useVIntl()

const messages = defineMessages({
	title: {
		id: 'app.guild-account.title',
		defaultMessage: 'Guild account',
	},
	description: {
		id: 'app.guild-account.description',
		defaultMessage:
			'Sign in to see guild announcements, modpacks and online members. Your guild account is separate from Modrinth.',
	},
	loading: {
		id: 'app.guild-account.loading',
		defaultMessage: 'Checking sign-in status…',
	},
	readFailed: {
		id: 'app.guild-account.read-failed',
		defaultMessage: 'Could not read sign-in status',
	},
	retryRead: {
		id: 'app.guild-account.retry-read',
		defaultMessage: 'Try again',
	},
	signedInAs: {
		id: 'app.guild-account.signed-in-as',
		defaultMessage: 'Signed in as',
	},
	signOut: {
		id: 'app.guild-account.sign-out',
		defaultMessage: 'Sign out',
	},
	signingOut: {
		id: 'app.guild-account.signing-out',
		defaultMessage: 'Signing out…',
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

const {
	session,
	busy,
	action,
	loading,
	readFailed,
	signInError,
	signOutFailed,
	refresh,
	signIn,
	signOut,
} = useGuildSession()

const statusMessage = computed(() => {
	if (loading.value) return messages.loading
	if (readFailed.value) return messages.readFailed
	return session.value ? messages.signedInAs : messages.notSignedIn
})

const actionMessage = computed(() => {
	if (action.value === 'signing-in') return messages.waiting
	if (action.value === 'signing-out') return messages.signingOut
	if (readFailed.value) return messages.retryRead
	return session.value ? messages.signOut : messages.signIn
})

const error = computed(() => {
	if (loading.value || readFailed.value || busy.value) return null
	if (!session.value) return signInError.value
	return signOutFailed.value ? formatMessage(messages.signOutFailed) : null
})

function handleAction() {
	if (readFailed.value) return refresh()
	return session.value ? signOut() : signIn()
}

onMounted(refresh)
</script>

<template>
	<div class="guild-account-container flex flex-col gap-4">
		<div class="flex flex-col gap-2">
			<span class="text-lg font-semibold text-contrast">{{ formatMessage(messages.title) }}</span>
			<span class="text-secondary">{{ formatMessage(messages.description) }}</span>
		</div>

		<div class="flex flex-col gap-3 rounded-2xl bg-bg p-4">
			<div class="account-row flex items-center justify-between gap-4">
				<div class="account-identity flex min-w-0 flex-col" role="status">
					<span class="text-sm" :class="readFailed && !loading ? 'text-red' : 'text-secondary'">
						{{ formatMessage(statusMessage) }}
					</span>
					<span v-if="session && !loading && !readFailed" class="font-semibold text-contrast">
						{{ session.name ?? session.user_id }}
					</span>
					<span v-if="busy" class="sr-only">{{ formatMessage(actionMessage) }}</span>
				</div>
				<Button class="account-action" :disabled="loading" :loading="busy" @click="handleAction">
					<SpinnerIcon
						v-if="busy"
						class="animate-spin motion-reduce:animate-none"
						aria-hidden="true"
					/>
					{{ formatMessage(actionMessage) }}
				</Button>
			</div>
			<p v-if="!session && !loading && !readFailed" class="m-0 text-sm text-secondary">
				{{ formatMessage(messages.signInHint) }}
			</p>
			<p v-if="error" class="account-error m-0 text-sm text-red" role="alert">{{ error }}</p>
		</div>
	</div>
</template>

<style scoped>
.guild-account-container {
	container-type: inline-size;
}

.account-identity,
.account-error,
.account-action {
	overflow-wrap: anywhere;
}

.account-action {
	height: auto;
	min-height: 2.25rem;
	max-width: 100%;
	flex-shrink: 0;
	padding-block: 0.375rem;
	white-space: normal;
	text-align: left;
}

.account-error {
	border-top: 1px solid var(--surface-5);
	padding-top: 0.75rem;
}

@container (max-width: 360px) {
	.account-row {
		flex-direction: column;
		align-items: flex-start;
		gap: 0.75rem;
	}

	.account-identity {
		width: 100%;
	}
}
</style>
