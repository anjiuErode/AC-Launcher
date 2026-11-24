<script setup lang="ts">
import { LogInIcon, SpinnerIcon } from '@modrinth/assets'
import { Button, commonMessages, defineMessages, Input, useVIntl } from '@modrinth/ui'
import { onBeforeUnmount, ref } from 'vue'

import {
	beginExternalLogin,
	chooseExternalProfile,
	discardExternalLogin,
	discoverExternalServer,
	type ExternalAuthServer,
	type ExternalProfile,
} from '@/helpers/external-auth'

const props = defineProps<{ initialAddress?: string }>()
const emit = defineEmits<{ created: []; cancel: [] }>()
const { formatMessage } = useVIntl()
const messages = defineMessages({
	address: { id: 'app.accounts.external.address', defaultMessage: 'HTTPS authentication server' },
	destination: {
		id: 'app.accounts.external.destination',
		defaultMessage: 'Your credentials will be sent to this server',
	},
	username: { id: 'app.accounts.external.username', defaultMessage: 'External account' },
	password: { id: 'app.accounts.external.password', defaultMessage: 'External account password' },
	login: { id: 'app.accounts.external.login', defaultMessage: 'Sign in to this server' },
	profile: { id: 'app.accounts.external.profile', defaultMessage: 'Choose a Minecraft profile' },
	discoveryFailed: {
		id: 'app.accounts.external.discovery-failed',
		defaultMessage: 'Could not verify this HTTPS authentication server.',
	},
	loginFailed: {
		id: 'app.accounts.external.login-failed',
		defaultMessage: 'Sign-in failed. Check your account, password, and authentication server.',
	},
	profileFailed: {
		id: 'app.accounts.external.profile-failed',
		defaultMessage: 'Could not activate the profile. Sign in and choose the profile again.',
	},
})

const address = ref(props.initialAddress ?? '')
const server = ref<ExternalAuthServer>()
const username = ref('')
const password = ref('')
const pending = ref<{ id: string; profiles: ExternalProfile[] }>()
const selectedProfile = ref('')
const busy = ref(false)
const error = ref('')
let unmounted = false

defineExpose({ busy })

onBeforeUnmount(() => {
	unmounted = true
	password.value = ''
	if (pending.value) void discardExternalLogin(pending.value.id).catch(() => undefined)
})

function reset() {
	if (pending.value) void discardExternalLogin(pending.value.id).catch(() => undefined)
	pending.value = undefined
	server.value = undefined
	selectedProfile.value = ''
	password.value = ''
	error.value = ''
}

function close() {
	reset()
	username.value = ''
	emit('cancel')
}

function complete() {
	reset()
	username.value = ''
	emit('created')
}

async function discover() {
	if (busy.value || !address.value.trim()) return
	busy.value = true
	error.value = ''
	try {
		server.value = await discoverExternalServer(address.value.trim())
	} catch {
		error.value = formatMessage(messages.discoveryFailed)
	} finally {
		busy.value = false
	}
}

async function signIn() {
	const destination = server.value
	if (busy.value || !destination || !username.value.trim() || !password.value) return
	const submittedPassword = password.value
	password.value = ''
	busy.value = true
	error.value = ''
	try {
		const result = await beginExternalLogin(
			destination.api_root,
			username.value.trim(),
			submittedPassword,
		)
		if (unmounted) {
			if (result.type === 'choose_profile') {
				await discardExternalLogin(result.login_id).catch(() => undefined)
			}
			return
		}
		if (result.type === 'ready') {
			complete()
		} else {
			pending.value = { id: result.login_id, profiles: result.profiles }
			selectedProfile.value = result.profiles.length === 1 ? result.profiles[0].id : ''
		}
	} catch {
		error.value = formatMessage(messages.loginFailed)
	} finally {
		busy.value = false
	}
}

async function selectProfile() {
	if (busy.value || !pending.value || !selectedProfile.value) return
	busy.value = true
	error.value = ''
	try {
		await chooseExternalProfile(pending.value.id, selectedProfile.value)
		if (!unmounted) complete()
	} catch {
		pending.value = undefined
		selectedProfile.value = ''
		error.value = formatMessage(messages.profileFailed)
	} finally {
		busy.value = false
	}
}
</script>

<template>
	<div class="flex flex-col gap-3 rounded-xl border border-solid border-surface-5 p-3">
		<form v-if="!server" autocomplete="off" class="flex flex-col gap-3" @submit.prevent="discover">
			<label class="flex flex-col gap-1">
				{{ formatMessage(messages.address) }}
				<Input
					v-model="address"
					placeholder="https://example.com/api/yggdrasil"
					autocomplete="off"
					:disabled="busy"
				/>
			</label>
			<div class="flex flex-wrap gap-2">
				<Button type="colored" color="brand" native-type="submit" :disabled="busy || !address.trim()">
					<SpinnerIcon v-if="busy" class="animate-spin" />
					{{ formatMessage(commonMessages.continueButton) }}
				</Button>
				<Button :disabled="busy" @click="close">{{
					formatMessage(commonMessages.cancelButton)
				}}</Button>
			</div>
		</form>
		<template v-else>
			<div class="text-sm">
				<p v-if="server.name" class="m-0 font-semibold text-contrast">{{ server.name }}</p>
				<p class="m-0">{{ formatMessage(messages.destination) }}</p>
				<p class="m-0 break-all text-contrast">{{ server.api_root }}</p>
			</div>
			<form v-if="!pending" autocomplete="off" class="flex flex-col gap-3" @submit.prevent="signIn">
				<label class="flex flex-col gap-1">
					{{ formatMessage(messages.username) }}
					<Input v-model="username" autocomplete="off" :disabled="busy" />
				</label>
				<label class="flex flex-col gap-1">
					{{ formatMessage(messages.password) }}
					<Input v-model="password" type="password" autocomplete="off" :disabled="busy" />
				</label>
				<div class="flex flex-wrap gap-2">
					<Button
						type="colored"
						color="brand"
					native-type="submit"
						:disabled="busy || !username.trim() || !password"
					>
						<SpinnerIcon v-if="busy" class="animate-spin" />
						<LogInIcon v-else />
						{{ formatMessage(messages.login) }}
					</Button>
					<Button :disabled="busy" @click="reset">{{
						formatMessage(commonMessages.cancelButton)
					}}</Button>
				</div>
			</form>
			<form v-else class="flex flex-col gap-2" @submit.prevent="selectProfile">
				<fieldset
					class="m-0 flex flex-col gap-2 rounded-lg border border-solid border-surface-5 p-2"
				>
					<legend>{{ formatMessage(messages.profile) }}</legend>
					<label
						v-for="profile in pending.profiles"
						:key="profile.id"
						class="flex items-center gap-2"
					>
						<input v-model="selectedProfile" type="radio" :value="profile.id" :disabled="busy" />
						{{ profile.name }}
					</label>
				</fieldset>
				<div class="flex flex-wrap gap-2">
					<Button
						type="colored"
						color="brand"
					native-type="submit"
						:disabled="busy || !selectedProfile"
					>
						<SpinnerIcon v-if="busy" class="animate-spin" />
						{{ formatMessage(commonMessages.continueButton) }}
					</Button>
					<Button :disabled="busy" @click="reset">{{
						formatMessage(commonMessages.cancelButton)
					}}</Button>
				</div>
			</form>
		</template>
		<p v-if="error" role="alert" class="m-0 text-sm text-red">{{ error }}</p>
	</div>
</template>
