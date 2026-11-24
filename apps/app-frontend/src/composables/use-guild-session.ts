import { invoke } from '@tauri-apps/api/core'
import { computed, ref } from 'vue'

export interface GuildSession {
	user_id: string
	name: string | null
}

/** Rust 侧 `guild::GuildApiError` 的序列化形态 */
export interface GuildApiError {
	code: 'not_signed_in' | 'unauthorized' | 'network' | 'server' | 'bad_response' | 'io'
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
const action = ref<'signing-in' | 'signing-out' | null>(null)
const busy = computed(() => action.value !== null)
const loading = ref(true)
const readFailed = ref(false)
const signInError = ref<string | null>(null)
const signOutFailed = ref(false)
let refreshTask: Promise<void> | null = null

async function loadSession(): Promise<void> {
	try {
		session.value = await invoke<GuildSession | null>('guild_session')
		readFailed.value = false
		if (session.value) signInError.value = null
		else signOutFailed.value = false
	} catch {
		readFailed.value = true
	}
}

function refresh(): Promise<void> {
	if (refreshTask) return refreshTask
	// 登录和退出已有确定的 IPC 结果，跳过写入期间的本地快照读取。
	if (busy.value) return Promise.resolve()
	loading.value = true
	refreshTask = loadSession().finally(() => {
		refreshTask = null
		loading.value = false
	})
	return refreshTask
}

/** 打开官网授权页并等待回环回调；返回是否登录成功 */
async function signIn(): Promise<boolean> {
	if (busy.value || loading.value || session.value) return false
	action.value = 'signing-in'
	signInError.value = null
	try {
		session.value = await invoke<GuildSession>('guild_begin_login')
		readFailed.value = false
		signOutFailed.value = false
		return true
	} catch (error) {
		signInError.value = describeGuildError(error, '登录失败，请重试')
		return false
	} finally {
		action.value = null
	}
}

async function signOut(): Promise<void> {
	if (busy.value || loading.value || !session.value) return
	action.value = 'signing-out'
	signOutFailed.value = false
	try {
		await invoke('guild_sign_out')
		session.value = null
		readFailed.value = false
	} catch {
		signOutFailed.value = true
	} finally {
		action.value = null
	}
}

export function useGuildSession() {
	return {
		session,
		action,
		busy,
		loading,
		readFailed,
		signInError,
		signOutFailed,
		signedIn: computed(() => session.value !== null),
		refresh,
		signIn,
		signOut,
	}
}
