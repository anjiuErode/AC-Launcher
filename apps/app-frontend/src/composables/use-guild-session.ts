import { invoke } from '@tauri-apps/api/core'
import { computed, ref } from 'vue'

export interface GuildSession {
	user_id: string
	name: string | null
}

/** Rust 侧 `guild::GuildApiError` 的序列化形态 */
export interface GuildApiError {
	code: 'not_signed_in' | 'unauthorized' | 'network' | 'server' | 'bad_response'
	message: string
}

export function isGuildApiError(error: unknown): error is GuildApiError {
	return (
		typeof error === 'object' &&
		error !== null &&
		'code' in error &&
		'message' in error &&
		typeof (error as GuildApiError).message === 'string'
	)
}

/** 把命令抛出的任意错误转成可展示的文案 */
export function describeGuildError(error: unknown, fallback: string): string {
	if (isGuildApiError(error) && error.message) return error.message
	if (typeof error === 'string' && error) return error
	return fallback
}

/**
 * 会话是模块级单例：侧边栏、公会页、设置页共用同一份状态。
 * 之前各页各自 `invoke('guild_session')`，设置页退出登录后侧边栏仍显示已登录，就是这个原因。
 */
const session = ref<GuildSession | null>(null)
const busy = ref(false)
const signInError = ref<string | null>(null)
const loaded = ref(false)

async function refresh(): Promise<void> {
	try {
		session.value = await invoke<GuildSession | null>('guild_session')
	} catch {
		session.value = null
	} finally {
		loaded.value = true
	}
}

/** 打开官网授权页并等待回环回调；返回是否登录成功 */
async function signIn(): Promise<boolean> {
	busy.value = true
	signInError.value = null
	try {
		session.value = await invoke<GuildSession>('guild_begin_login')
		return true
	} catch (error) {
		signInError.value = describeGuildError(error, '登录失败，请重试')
		return false
	} finally {
		busy.value = false
	}
}

async function signOut(): Promise<void> {
	try {
		await invoke('guild_sign_out')
	} finally {
		session.value = null
	}
}

export function useGuildSession() {
	return {
		session,
		busy,
		loaded,
		signInError,
		signedIn: computed(() => session.value !== null),
		refresh,
		signIn,
		signOut,
	}
}
