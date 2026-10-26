<script setup lang="ts">
import { Button, defineMessages, useVIntl } from '@modrinth/ui'
import { invoke } from '@tauri-apps/api/core'
import { onMounted, ref } from 'vue'

interface GuildSession {
	user_id: string
	email: string | null
	expires_at: number
}

const { formatMessage } = useVIntl()

const messages = defineMessages({
	title: {
		id: 'app.guild-account.title',
		defaultMessage: 'Guild account',
	},
	description: {
		id: 'app.guild-account.description',
		defaultMessage:
			'Signs you in to guild-only content: announcements, modpacks and the member list. Separate from your Modrinth account.',
	},
	signedInAs: {
		id: 'app.guild-account.signed-in-as',
		defaultMessage: 'Signed in as',
	},
	signOut: {
		id: 'app.guild-account.sign-out',
		defaultMessage: 'Sign out',
	},
	email: {
		id: 'app.guild-account.email',
		defaultMessage: 'Email',
	},
	password: {
		id: 'app.guild-account.password',
		defaultMessage: 'Password',
	},
	signIn: {
		id: 'app.guild-account.sign-in',
		defaultMessage: 'Sign in',
	},
	signUp: {
		id: 'app.guild-account.sign-up',
		defaultMessage: 'Create account',
	},
	toSignUp: {
		id: 'app.guild-account.to-sign-up',
		defaultMessage: "Don't have an account? Create one",
	},
	toSignIn: {
		id: 'app.guild-account.to-sign-in',
		defaultMessage: 'Already have an account? Sign in',
	},
	working: {
		id: 'app.guild-account.working',
		defaultMessage: 'Working…',
	},
})

const session = ref<GuildSession | null>(null)
const loaded = ref(false)
const busy = ref(false)
const mode = ref<'sign-in' | 'sign-up'>('sign-in')
const email = ref('')
const password = ref('')
const notice = ref<string | null>(null)
const error = ref<string | null>(null)

async function refresh() {
	session.value = await invoke<GuildSession | null>('guild_session')
}

onMounted(async () => {
	try {
		await refresh()
	} catch (err) {
		error.value = String(err)
	} finally {
		loaded.value = true
	}
})

async function submit() {
	busy.value = true
	error.value = null
	notice.value = null
	try {
		if (mode.value === 'sign-in') {
			session.value = await invoke<GuildSession>('guild_sign_in', {
				email: email.value,
				password: password.value,
			})
			password.value = ''
		} else {
			notice.value = await invoke<string>('guild_sign_up', {
				email: email.value,
				password: password.value,
			})
			password.value = ''
			await refresh()
		}
	} catch (err) {
		error.value = String(err)
	} finally {
		busy.value = false
	}
}

async function signOut() {
	busy.value = true
	error.value = null
	notice.value = null
	try {
		await invoke('guild_sign_out')
		session.value = null
	} catch (err) {
		error.value = String(err)
	} finally {
		busy.value = false
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
				<span class="font-semibold text-contrast">{{ session.email ?? session.user_id }}</span>
			</span>
			<Button :disabled="busy" @click="signOut">{{ formatMessage(messages.signOut) }}</Button>
		</div>

		<form v-else class="flex flex-col gap-3 rounded-2xl bg-bg p-4" @submit.prevent="submit">
			<label class="flex flex-col gap-1">
				<span class="text-sm text-secondary">{{ formatMessage(messages.email) }}</span>
				<input
					v-model="email"
					type="email"
					required
					autocomplete="username"
					class="w-full rounded-xl bg-bg-raised px-4 py-3 text-contrast outline-none"
				/>
			</label>
			<label class="flex flex-col gap-1">
				<span class="text-sm text-secondary">{{ formatMessage(messages.password) }}</span>
				<input
					v-model="password"
					type="password"
					required
					minlength="6"
					autocomplete="current-password"
					class="w-full rounded-xl bg-bg-raised px-4 py-3 text-contrast outline-none"
				/>
			</label>
			<div class="flex items-center gap-2">
				<Button :disabled="busy || !loaded" @click="submit">
					{{
						busy
							? formatMessage(messages.working)
							: mode === 'sign-in'
								? formatMessage(messages.signIn)
								: formatMessage(messages.signUp)
					}}
				</Button>
				<button
					type="button"
					class="cursor-pointer border-none bg-transparent text-sm text-secondary underline"
					@click="mode = mode === 'sign-in' ? 'sign-up' : 'sign-in'"
				>
					{{
						mode === 'sign-in'
							? formatMessage(messages.toSignUp)
							: formatMessage(messages.toSignIn)
					}}
				</button>
			</div>
		</form>

		<p v-if="notice" class="m-0 text-sm text-secondary">{{ notice }}</p>
		<p v-if="error" class="m-0 text-sm text-red">{{ error }}</p>
	</div>
</template>
