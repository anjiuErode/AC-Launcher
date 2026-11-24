import { invoke } from '@tauri-apps/api/core'

export type ExternalAuthServer = {
	api_root: string
	name: string | null
}

export type ExternalProfile = {
	id: string
	name: string
}

export type ExternalAccount = {
	kind: 'external'
	server: ExternalAuthServer
	profile: ExternalProfile
}

type ExternalLogin =
	| { type: 'ready' }
	| { type: 'choose_profile'; login_id: string; profiles: ExternalProfile[] }

const AUTHLIB_CARD_PREFIX = 'authlib-injector:yggdrasil-server:'

export function parseExternalAuthLink(text: string): string | undefined {
	const card = text.trim()
	if (card.length > 8192 || !card.startsWith(AUTHLIB_CARD_PREFIX)) return

	try {
		const address = decodeURIComponent(card.slice(AUTHLIB_CARD_PREFIX.length))
		const authority = /^https:\/\/([^/]+)/i.exec(address)?.[1]
		if (!authority || authority.includes('@') || /[\s\\?#]/.test(address)) return
		const url = new URL(address)
		if (url.username || url.password || url.href.length > 2048) return
		return url.href
	} catch {
		return undefined
	}
}

export function getExternalAccounts(): Promise<ExternalAccount[]> {
	return invoke('plugin:auth|get_external_users')
}

export function discoverExternalServer(address: string): Promise<ExternalAuthServer> {
	return invoke('plugin:auth|discover_external_server', { address })
}

export function beginExternalLogin(
	apiRoot: string,
	username: string,
	password: string,
): Promise<ExternalLogin> {
	return invoke('plugin:auth|begin_external_login', { apiRoot, username, password })
}

export function chooseExternalProfile(loginId: string, profileId: string): Promise<void> {
	return invoke('plugin:auth|choose_external_profile', { loginId, profileId })
}

export function discardExternalLogin(loginId: string): Promise<void> {
	return invoke('plugin:auth|discard_external_login', { loginId })
}
