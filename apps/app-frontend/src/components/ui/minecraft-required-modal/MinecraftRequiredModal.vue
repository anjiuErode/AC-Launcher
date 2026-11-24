<script setup lang="ts">
import {
	ArrowDownIcon,
	MicrosoftColorIcon,
	ServerIcon,
	SpinnerIcon,
	UserIcon,
} from '@modrinth/assets'
import { defineMessages, NewModal, useVIntl } from '@modrinth/ui'
import { isTauri } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { computed, nextTick, onMounted, onUnmounted, ref } from 'vue'

import { handleSevereError } from '@/composables/use-error.js'
import { injectMinecraftAccounts } from '@/features/minecraft-accounts/context'
import ExternalAccountForm from '@/features/minecraft-accounts/external-account-form.vue'
import OfflineAccountForm from '@/features/minecraft-accounts/offline-account-form.vue'
import { trackEvent } from '@/helpers/analytics'
import { login as loginFlow, set_default_user } from '@/helpers/auth.js'
import { parseExternalAuthLink } from '@/helpers/external-auth'

type LoginMethod = 'microsoft' | 'offline' | 'external'

const { formatMessage } = useVIntl()
const minecraftAccounts = injectMinecraftAccounts()

const messages = defineMessages({
	header: {
		id: 'minecraft-required.header',
		defaultMessage: 'Choose a sign-in method',
	},
	microsoft: {
		id: 'minecraft-required.microsoft',
		defaultMessage: 'Microsoft sign-in',
	},
	microsoftDescription: {
		id: 'minecraft-required.microsoft-description',
		defaultMessage: 'Verify your Minecraft purchase and use official online services.',
	},
	offline: {
		id: 'minecraft-required.offline',
		defaultMessage: 'Offline sign-in',
	},
	offlineDescription: {
		id: 'minecraft-required.offline-description',
		defaultMessage: 'Enter a player name to play locally.',
	},
	external: {
		id: 'minecraft-required.external',
		defaultMessage: 'Third-party sign-in',
	},
	externalDescription: {
		id: 'minecraft-required.external-description',
		defaultMessage: 'Sign in with your account from a third-party authentication server.',
	},
	dropHint: {
		id: 'minecraft-required.drop-hint',
		defaultMessage: '(Or drag a website card to sign in)',
	},
	invalidDrop: {
		id: 'minecraft-required.invalid-drop',
		defaultMessage: 'Drop an Authlib-Injector sign-in card from an authentication website.',
	},
})

const methods = [
	{
		id: 'microsoft',
		title: messages.microsoft,
		description: messages.microsoftDescription,
		icon: MicrosoftColorIcon,
	},
	{
		id: 'offline',
		title: messages.offline,
		description: messages.offlineDescription,
		icon: UserIcon,
	},
	{
		id: 'external',
		title: messages.external,
		description: messages.externalDescription,
		icon: ServerIcon,
	},
] as const

const modal = ref<InstanceType<typeof NewModal>>()
const isOpen = ref(false)
const chooser = ref<HTMLElement>()
const formHeading = ref<HTMLElement>()
const form = ref<{ busy: boolean }>()
const dropZone = ref<HTMLElement>()
const dragOver = ref(false)
const dropError = ref(false)
const method = ref<Exclude<LoginMethod, 'microsoft'>>()
const externalAddress = ref('')
const loadingSignIn = ref(false)
const refreshingAccounts = ref(false)
const busy = computed(
	() => loadingSignIn.value || Boolean(form.value?.busy) || refreshingAccounts.value,
)
const microsoftBusy = computed(() => loadingSignIn.value || minecraftAccounts.value?.loginDisabled)
const canChooseMethod = computed(
	() => isOpen.value && !method.value && !busy.value && !microsoftBusy.value,
)

function show() {
	if (isOpen.value || !modal.value) return
	isOpen.value = true
	modal.value.show()
}

function reset() {
	isOpen.value = false
	method.value = undefined
	externalAddress.value = ''
	dragOver.value = false
	dropError.value = false
}

function backToChooser() {
	const previous = method.value
	method.value = undefined
	externalAddress.value = ''
	void nextTick(() => {
		chooser.value
			?.querySelector<HTMLButtonElement>('[data-login-method="' + previous + '"]')
			?.focus()
	})
}

function chooseMethod(nextMethod: LoginMethod, address = '') {
	if (!canChooseMethod.value) return
	dragOver.value = false
	dropError.value = false
	if (nextMethod === 'microsoft') {
		void signInWithMicrosoft()
		return
	}
	externalAddress.value = address
	method.value = nextMethod
	void nextTick(() => formHeading.value?.focus())
}

async function alternateAccountCreated() {
	refreshingAccounts.value = true
	try {
		await minecraftAccounts.value?.refreshValues()
	} finally {
		refreshingAccounts.value = false
	}
	await nextTick()
	modal.value?.hide()
}

async function signInWithMicrosoft() {
	const accounts = minecraftAccounts.value
	loadingSignIn.value = true
	accounts?.setLoginDisabled(true)
	try {
		const loggedIn = await loginFlow()
		if (!loggedIn) return
		await set_default_user(loggedIn.profile.id)
		await accounts?.refreshValues()
		await trackEvent('AccountLogIn', { source: 'MinecraftRequiredModal' })
	} catch (error) {
		handleSevereError(error)
		return
	} finally {
		loadingSignIn.value = false
		accounts?.setLoginDisabled(false)
	}
	await nextTick()
	modal.value?.hide()
}

function hasFiles(transfer: DataTransfer) {
	return transfer.types.includes('Files') || transfer.files.length > 0
}

function onDragOver(event: DragEvent) {
	const transfer = event.dataTransfer
	if (!transfer) return
	dragOver.value =
		canChooseMethod.value &&
		!hasFiles(transfer) &&
		(transfer.types.includes('text/uri-list') || transfer.types.includes('text/plain'))
	transfer.dropEffect = dragOver.value ? 'copy' : 'none'
}

function acceptDroppedLink(text: string) {
	if (!canChooseMethod.value) return
	const address = parseExternalAuthLink(text)
	if (!address) {
		dropError.value = true
		return
	}
	chooseMethod('external', address)
}

function onDrop(event: DragEvent) {
	dragOver.value = false
	const transfer = event.dataTransfer
	if (!transfer || hasFiles(transfer)) return
	const uriList = transfer.getData('text/uri-list')
	if (uriList) {
		const links = uriList
			.split(/\r?\n/)
			.map((line) => line.trim())
			.filter((line) => line && !line.startsWith('#'))
		acceptDroppedLink(links.length === 1 ? links[0] : '')
	} else {
		acceptDroppedLink(transfer.getData('text/plain'))
	}
}

let nativeDropListener: Promise<UnlistenFn | void> | undefined
onMounted(() => {
	if (!isTauri()) return
	nativeDropListener = listen<{ text: string; position: { x: number; y: number } }>(
		'acl-auth-card-drop',
		({ payload }) => {
			const zone = dropZone.value
			if (!canChooseMethod.value || !zone) return
			const x = payload.position.x / window.devicePixelRatio
			const y = payload.position.y / window.devicePixelRatio
			if (zone.contains(zone.ownerDocument.elementFromPoint(x, y))) {
				acceptDroppedLink(payload.text)
			}
		},
	).catch((error) => console.warn('Unable to listen for native authentication card drops:', error))
})
onUnmounted(() => {
	void nativeDropListener?.then((unlisten) => unlisten?.())
})

defineExpose({ show })
</script>

<template>
	<NewModal
		ref="modal"
		:header="formatMessage(messages.header)"
		:initial-focus="() => chooser?.querySelector('button') ?? null"
		:disable-close="busy"
		width="680px"
		max-width="680px"
		scrollable
		@hide="reset"
	>
		<div v-if="!method" ref="chooser" class="flex flex-col gap-4 pb-5">
			<div class="grid grid-cols-1 gap-3 sm:grid-cols-3">
				<button
					v-for="option in methods"
					:key="option.id"
					type="button"
					:data-login-method="option.id"
					:aria-labelledby="'minecraft-login-title-' + option.id"
					:aria-describedby="'minecraft-login-description-' + option.id"
					:aria-busy="option.id === 'microsoft' && microsoftBusy ? true : undefined"
					:disabled="!canChooseMethod"
					class="flex min-w-0 cursor-pointer items-center gap-3 rounded-xl border border-solid border-surface-5 bg-button-bg p-4 text-left text-primary transition-colors hover:border-brand hover:bg-surface-3 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-brand disabled:cursor-default disabled:opacity-60 sm:min-h-[146px] sm:flex-col sm:items-start"
					@click="chooseMethod(option.id)"
				>
					<span
						class="flex size-9 shrink-0 items-center justify-center rounded-lg bg-surface-4 text-contrast"
						aria-hidden="true"
					>
						<SpinnerIcon
							v-if="option.id === 'microsoft' && microsoftBusy"
							class="size-5 motion-safe:animate-spin"
						/>
						<component :is="option.icon" v-else class="size-5" />
					</span>
					<span class="flex min-w-0 flex-col gap-1">
						<span
							:id="'minecraft-login-title-' + option.id"
							class="text-base font-semibold text-contrast"
						>
							{{ formatMessage(option.title) }}
						</span>
						<span
							:id="'minecraft-login-description-' + option.id"
							class="text-sm leading-5 text-secondary"
						>
							{{ formatMessage(option.description) }}
						</span>
					</span>
				</button>
			</div>
			<div
				ref="dropZone"
				class="flex min-h-16 items-center justify-center gap-2 rounded-xl border border-dashed px-3 py-2 text-center text-sm transition-colors"
				:class="
					dragOver
						? 'border-brand bg-surface-3 text-contrast'
						: 'border-surface-5 bg-surface-2 text-secondary'
				"
				@dragenter.prevent="onDragOver"
				@dragover.prevent="onDragOver"
				@dragleave="dragOver = false"
				@drop.prevent="onDrop"
			>
				<ArrowDownIcon class="pointer-events-none size-4 shrink-0" aria-hidden="true" />
				<span class="pointer-events-none">{{ formatMessage(messages.dropHint) }}</span>
			</div>
			<p v-if="dropError" role="status" class="m-0 text-sm text-red">
				{{ formatMessage(messages.invalidDrop) }}
			</p>
		</div>
		<div v-else class="flex flex-col gap-3 pb-5">
			<h2
				ref="formHeading"
				tabindex="-1"
				class="m-0 text-lg font-semibold text-contrast outline-none"
			>
				{{ formatMessage(method === 'offline' ? messages.offline : messages.external) }}
			</h2>
			<OfflineAccountForm
				v-if="method === 'offline'"
				ref="form"
				@created="alternateAccountCreated"
				@cancel="backToChooser"
			/>
			<ExternalAccountForm
				v-else
				ref="form"
				:initial-address="externalAddress"
				@created="alternateAccountCreated"
				@cancel="backToChooser"
			/>
		</div>
	</NewModal>
</template>
