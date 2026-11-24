import { createContext } from '@modrinth/ui'
import { useLocalStorage } from '@vueuse/core'
import { computed, type ComputedRef, ref, watch } from 'vue'

import { useAppEvent } from '@/composables/use-app-event'
import { useGuildSession } from '@/composables/use-guild-session'
import { getOnboardingChecklist, type OnboardingChecklist } from '@/helpers/onboarding-checklist'
import type { AppEvents } from '@/providers/app-events'

export interface OnboardingChecklistContext {
	hasCreatedInstance: ComputedRef<boolean>
	hasLoggedIntoMinecraft: ComputedRef<boolean>
	hasLoggedIntoGuild: ComputedRef<boolean>
	isReady: ComputedRef<boolean>
	showChecklist: ComputedRef<boolean>
	showModrinthFriends: ComputedRef<boolean>
}

export interface OnboardingChecklistProvider extends OnboardingChecklistContext {
	initialize: () => Promise<void>
}

export const [injectOnboardingChecklist, provideOnboardingChecklist] =
	createContext<OnboardingChecklistContext>('root', 'onboardingChecklist')

export function setupOnboardingChecklistProvider(
	appEvents: AppEvents,
): OnboardingChecklistProvider {
	const checklist = ref<OnboardingChecklist>()
	const { loading, signedIn } = useGuildSession()
	const guildSessionReady = ref(false)
	const guildSignInCompleted = useLocalStorage('acl:onboarding:guild-sign-in-completed', false)
	watch(
		loading,
		(isLoading) => {
			if (!isLoading) guildSessionReady.value = true
		},
		{ immediate: true },
	)
	watch(
		signedIn,
		(isSignedIn) => {
			if (isSignedIn) guildSignInCompleted.value = true
		},
		{ immediate: true, flush: 'sync' },
	)
	const hasLoggedIntoGuild = computed(() => signedIn.value || guildSignInCompleted.value)

	const context: OnboardingChecklistContext = {
		hasCreatedInstance: computed(() => checklist.value?.has_created_instance ?? false),
		hasLoggedIntoMinecraft: computed(() => checklist.value?.has_logged_into_minecraft ?? false),
		hasLoggedIntoGuild,
		isReady: computed(() => checklist.value !== undefined),
		showChecklist: computed(() => {
			const current = checklist.value
			return (
				guildSessionReady.value &&
				!!current &&
				(!current.has_created_instance ||
					!current.has_logged_into_minecraft ||
					!hasLoggedIntoGuild.value)
			)
		}),
		showModrinthFriends: computed(
			() => !checklist.value?.show_checklist || checklist.value.has_logged_into_modrinth,
		),
	}

	provideOnboardingChecklist(context)

	const applyChecklist = (nextChecklist: OnboardingChecklist) => {
		checklist.value = {
			has_created_instance:
				(checklist.value?.has_created_instance ?? false) || nextChecklist.has_created_instance,
			has_logged_into_minecraft:
				(checklist.value?.has_logged_into_minecraft ?? false) ||
				nextChecklist.has_logged_into_minecraft,
			has_logged_into_modrinth:
				(checklist.value?.has_logged_into_modrinth ?? false) ||
				nextChecklist.has_logged_into_modrinth,
			show_checklist: (checklist.value?.show_checklist ?? true) && nextChecklist.show_checklist,
		}
	}

	useAppEvent('onboarding_checklist', applyChecklist, appEvents)

	const initialize = async () => applyChecklist(await getOnboardingChecklist())

	return { ...context, initialize }
}
