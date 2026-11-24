import { invoke } from '@tauri-apps/api/core'
import { readonly, ref } from 'vue'

interface AclHotfix {
	version: string
	baseVersion: string
	notes: string | null
	releaseUrl: string
}

type HotfixCheckResult =
	| { status: 'unsupported' | 'unavailable' }
	| { status: 'available'; hotfix: AclHotfix }

const state = ref<HotfixCheckResult | { status: 'idle' | 'checking' | 'failed' }>({ status: 'idle' })
let inFlight: Promise<void> | null = null

export const aclHotfixState = readonly(state)

export function checkAclHotfix(): Promise<void> {
	if (inFlight) return inFlight

	state.value = { status: 'checking' }
	inFlight = invoke<HotfixCheckResult>('check_acl_hotfix')
		.then((result) => {
			state.value = result
		})
		.catch(() => {
			state.value = { status: 'failed' }
		})
		.finally(() => {
			inFlight = null
		})
	return inFlight
}
