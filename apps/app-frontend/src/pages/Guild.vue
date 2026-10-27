<script setup lang="ts">
import {
	CheckIcon,
	CopyIcon,
	MegaphoneIcon,
	PackageIcon,
	RefreshCwIcon,
	ServerIcon,
	SpinnerIcon,
} from '@modrinth/assets'
import { Button, defineMessages, Tabs, useVIntl } from '@modrinth/ui'
import { renderString } from '@modrinth/utils/parse'
import { invoke } from '@tauri-apps/api/core'
import { openUrl } from '@tauri-apps/plugin-opener'
import type { Ref } from 'vue'
import { computed, onMounted, reactive, ref } from 'vue'

import {
	describeGuildError,
	isGuildApiError,
	useGuildSession,
} from '@/composables/use-guild-session'

interface GuildMe {
	user_id: string
	username: string
	role: string
	joined_at: string | null
}

interface GuildAttachment {
	id: unknown
	name: string
	size: number
	mime: string
}

interface GuildAnnouncement {
	id: unknown
	title: string
	body: string
	pinned: boolean
	published_at: string | null
	author: string
	attachments?: GuildAttachment[]
}

interface GuildPack {
	id: unknown
	name: string
	mc_version: string
	loader: string
	description: string
	url: string
	sha256: string
	size_bytes: number
}

interface GuildServer {
	id: unknown
	name: string
	host: string
	port: number
	description: string
	whitelist_required: boolean
}

type SectionKey = 'announcements' | 'packs' | 'servers'

/** 与 Rust 侧 API_BASE 同源；附件是公开直链，前端直接取 */
const API_BASE = 'https://yingzhou1110.top'

const { formatMessage } = useVIntl()
const { session, busy, signIn, refresh } = useGuildSession()

const me = ref<GuildMe | null>(null)
const meError = ref<string | null>(null)
const meLoading = ref(false)

const announcements = ref<GuildAnnouncement[]>([])
const packs = ref<GuildPack[]>([])
const servers = ref<GuildServer[]>([])

const loading = reactive<Record<SectionKey, boolean>>({
	announcements: false,
	packs: false,
	servers: false,
})
const errors = reactive<Record<SectionKey, string | null>>({
	announcements: null,
	packs: null,
	servers: null,
})

const activeTab = ref('announcements')
const copiedKey = ref<string | null>(null)

const messages = defineMessages({
	title: {
		id: 'app.guild.title',
		defaultMessage: 'Guild',
	},
	subtitle: {
		id: 'app.guild.subtitle',
		defaultMessage: 'Announcements, modpacks and servers of the guild.',
	},
	refresh: {
		id: 'app.guild.refresh',
		defaultMessage: 'Refresh',
	},
	signedOutTitle: {
		id: 'app.guild.signed-out-title',
		defaultMessage: 'Not signed in to the guild',
	},
	signedOutDescription: {
		id: 'app.guild.signed-out-description',
		defaultMessage:
			'Sign in with your yingzhou1110.top account to read guild announcements and get the modpacks and servers.',
	},
	signIn: {
		id: 'app.guild.sign-in',
		defaultMessage: 'Sign in to guild account',
	},
	waiting: {
		id: 'app.guild.waiting',
		defaultMessage: 'Waiting for the browser…',
	},
	unavailable: {
		id: 'app.guild.unavailable',
		defaultMessage: 'Guild data is temporarily unavailable',
	},
	retry: {
		id: 'app.guild.retry',
		defaultMessage: 'Retry',
	},
	loading: {
		id: 'app.guild.loading',
		defaultMessage: 'Loading…',
	},
	tabAnnouncements: {
		id: 'app.guild.tab.announcements',
		defaultMessage: 'Announcements',
	},
	tabPacks: {
		id: 'app.guild.tab.packs',
		defaultMessage: 'Modpacks',
	},
	tabServers: {
		id: 'app.guild.tab.servers',
		defaultMessage: 'Servers',
	},
	pinned: {
		id: 'app.guild.pinned',
		defaultMessage: 'Pinned',
	},
	emptyAnnouncements: {
		id: 'app.guild.empty.announcements',
		defaultMessage: 'No announcements yet',
	},
	emptyPacks: {
		id: 'app.guild.empty.packs',
		defaultMessage: 'No modpacks yet',
	},
	emptyServers: {
		id: 'app.guild.empty.servers',
		defaultMessage: 'No servers yet',
	},
	download: {
		id: 'app.guild.pack.download',
		defaultMessage: 'Download',
	},
	size: {
		id: 'app.guild.pack.size',
		defaultMessage: 'Size',
	},
	checksum: {
		id: 'app.guild.pack.checksum',
		defaultMessage: 'SHA-256',
	},
	attachmentTitle: {
		id: 'app.guild.attachment.title',
		defaultMessage: 'Attachments',
	},
	attachmentOpen: {
		id: 'app.guild.attachment.open',
		defaultMessage: 'Open in browser',
	},
	attachmentPreview: {
		id: 'app.guild.attachment.preview',
		defaultMessage: 'Preview',
	},
	attachmentHide: {
		id: 'app.guild.attachment.hide',
		defaultMessage: 'Hide preview',
	},
	attachmentPreviewLoading: {
		id: 'app.guild.attachment.preview-loading',
		defaultMessage: 'Loading…',
	},
	attachmentPreviewFailed: {
		id: 'app.guild.attachment.preview-failed',
		defaultMessage: 'This attachment cannot be previewed here',
	},
	copyAddress: {
		id: 'app.guild.server.copy',
		defaultMessage: 'Copy address',
	},
	copied: {
		id: 'app.guild.server.copied',
		defaultMessage: 'Copied',
	},
	whitelist: {
		id: 'app.guild.server.whitelist',
		defaultMessage: 'Whitelist required',
	},
	roleAdmin: {
		id: 'app.guild.role.admin',
		defaultMessage: 'Admin',
	},
	roleMember: {
		id: 'app.guild.role.member',
		defaultMessage: 'Member',
	},
})

const tabs = computed(() => [
	{
		value: 'announcements',
		label: formatMessage(messages.tabAnnouncements),
		icon: MegaphoneIcon,
	},
	{ value: 'packs', label: formatMessage(messages.tabPacks), icon: PackageIcon },
	{
		value: 'servers',
		label: formatMessage(messages.tabServers),
		icon: ServerIcon,
	},
])

const displayName = computed(
	() => session.value?.name || me.value?.username || session.value?.user_id || '',
)

const roleLabel = computed(() => {
	if (!me.value) return ''
	return me.value.role === 'admin'
		? formatMessage(messages.roleAdmin)
		: formatMessage(messages.roleMember)
})

/** 会话失效后本地登录态已被 Rust 侧清除，这里同步刷新让界面回到未登录态 */
async function handleRequestError(error: unknown, fallback: string): Promise<string> {
	if (isGuildApiError(error) && error.code === 'unauthorized') {
		await refresh()
	}
	return describeGuildError(error, fallback)
}

async function loadMe(): Promise<void> {
	if (!session.value) return
	meLoading.value = true
	meError.value = null
	try {
		me.value = await invoke<GuildMe>('guild_me')
	} catch (error) {
		meError.value = await handleRequestError(
			error,
			formatMessage(messages.unavailable),
		)
	} finally {
		meLoading.value = false
	}
}

async function loadList<T>(
	command: string,
	target: Ref<T[]>,
	section: SectionKey,
	args?: Record<string, unknown>,
): Promise<void> {
	loading[section] = true
	errors[section] = null
	try {
		target.value = await invoke<T[]>(command, args)
	} catch (error) {
		errors[section] = await handleRequestError(
			error,
			formatMessage(messages.unavailable),
		)
	} finally {
		loading[section] = false
	}
}

async function loadAll(): Promise<void> {
	if (!session.value) return
	await Promise.all([
		loadMe(),
		loadList('guild_announcements', announcements, 'announcements', { limit: 20 }),
		loadList('guild_packs', packs, 'packs'),
		loadList('guild_servers', servers, 'servers'),
	])
}

function resetAll(): void {
	me.value = null
	meError.value = null
	announcements.value = []
	packs.value = []
	servers.value = []
	errors.announcements = null
	errors.packs = null
	errors.servers = null
}

async function handleSignIn(): Promise<void> {
	if (await signIn()) await loadAll()
}

async function handleRefresh(): Promise<void> {
	await refresh()
	if (session.value) await loadAll()
	else resetAll()
}

onMounted(async () => {
	await refresh()
	if (session.value) await loadAll()
})

function formatDate(value: string | null): string {
	if (!value) return ''
	const date = new Date(value)
	if (Number.isNaN(date.getTime())) return ''
	return date.toLocaleString(undefined, {
		year: 'numeric',
		month: 'short',
		day: 'numeric',
		hour: '2-digit',
		minute: '2-digit',
	})
}

function formatSize(bytes: number): string {
	if (!bytes) return ''
	const megabytes = bytes / 1024 / 1024
	return `${megabytes >= 100 ? megabytes.toFixed(0) : megabytes.toFixed(1)} MB`
}

type AttachmentKind = 'image' | 'markdown' | 'text' | 'pdf' | null

/** 按 MIME 判类型，MIME 缺失时退回扩展名 */
function attachmentKind(attachment: GuildAttachment): AttachmentKind {
	const mime = attachment.mime.toLowerCase()
	const name = attachment.name.toLowerCase()
	if (mime.startsWith('image/') || /\.(png|jpe?g|gif|webp)$/.test(name)) return 'image'
	if (mime === 'application/pdf' || name.endsWith('.pdf')) return 'pdf'
	if (mime === 'text/markdown' || name.endsWith('.md')) return 'markdown'
	if (mime === 'text/plain' || name.endsWith('.txt')) return 'text'
	return null
}

/** 每份附件的内联预览状态，键为 newsId:attachmentId */
const previews = reactive<
	Record<string, { open: boolean; loading: boolean; text: string | null; error: string | null }>
>({})

function previewKey(announcement: GuildAnnouncement, attachment: GuildAttachment): string {
	return `${String(announcement.id)}:${String(attachment.id)}`
}

function previewOf(announcement: GuildAnnouncement, attachment: GuildAttachment) {
	return previews[previewKey(announcement, attachment)]
}

/** 文本类附件才需要拉内容；PDF 直接交给 webview 的 iframe */
async function togglePreview(
	announcement: GuildAnnouncement,
	attachment: GuildAttachment,
): Promise<void> {
	const key = previewKey(announcement, attachment)
	const entry = previews[key] ?? { open: false, loading: false, text: null, error: null }
	previews[key] = entry
	entry.open = !entry.open
	if (!entry.open || entry.text !== null || entry.loading) return

	const kind = attachmentKind(attachment)
	if (kind !== 'markdown' && kind !== 'text') return

	entry.loading = true
	entry.error = null
	try {
		entry.text = await invoke<string>('guild_attachment_text', {
			newsId: String(announcement.id),
			attachmentId: String(attachment.id),
		})
	} catch (error) {
		entry.error = describeGuildError(error, formatMessage(messages.attachmentPreviewFailed))
	} finally {
		entry.loading = false
	}
}

/** 附件内容走官网公开下载地址，不需要令牌；图片直接内联，其余交给浏览器打开 */
function attachmentUrl(announcement: GuildAnnouncement, attachment: GuildAttachment): string {
	const newsId = encodeURIComponent(String(announcement.id))
	const attachmentId = encodeURIComponent(String(attachment.id))
	return `${API_BASE}/api/news/${newsId}/attachments/${attachmentId}`
}

async function openAttachment(
	announcement: GuildAnnouncement,
	attachment: GuildAttachment,
): Promise<void> {
	try {
		await openUrl(attachmentUrl(announcement, attachment))
	} catch (error) {
		console.error('failed to open attachment', error)
	}
}

function addressOf(server: GuildServer): string {
	return `${server.host}:${server.port}`
}

async function copyText(text: string, key: string): Promise<void> {
	try {
		await navigator.clipboard.writeText(text)
		copiedKey.value = key
		window.setTimeout(() => {
			if (copiedKey.value === key) copiedKey.value = null
		}, 1500)
	} catch (error) {
		console.error('failed to copy', error)
	}
}

async function openPack(pack: GuildPack): Promise<void> {
	if (!pack.url) return
	try {
		await openUrl(pack.url)
	} catch (error) {
		console.error('failed to open download link', error)
	}
}
</script>

<template>
	<div class="box-border h-full overflow-y-auto p-6">
		<div class="mx-auto flex w-full max-w-4xl flex-col gap-6">
			<header class="flex flex-wrap items-start justify-between gap-4">
				<div class="flex flex-col gap-1">
					<h1 class="m-0 text-2xl font-extrabold text-contrast">
						{{ formatMessage(messages.title) }}
					</h1>
					<p class="m-0 text-sm text-secondary">
						{{ formatMessage(messages.subtitle) }}
					</p>
				</div>
				<div v-if="session" class="flex items-center gap-3">
					<div class="flex flex-col items-end">
						<span class="text-sm font-semibold text-contrast">{{ displayName }}</span>
						<span v-if="meLoading" class="text-xs text-secondary">
							{{ formatMessage(messages.loading) }}
						</span>
						<span v-else-if="roleLabel" class="text-xs text-secondary">
							{{ roleLabel }}
						</span>
					</div>
					<Button type="outlined" :disabled="meLoading" @click="handleRefresh">
						<RefreshCwIcon class="size-4" :class="{ 'animate-spin': meLoading }" />
						{{ formatMessage(messages.refresh) }}
					</Button>
				</div>
			</header>

			<section
				v-if="!session"
				class="flex flex-col items-center gap-3 rounded-2xl border border-solid border-surface-5 bg-bg-raised p-8 text-center"
			>
				<h2 class="m-0 text-lg font-bold text-contrast">
					{{ formatMessage(messages.signedOutTitle) }}
				</h2>
				<p class="m-0 max-w-prose text-sm text-secondary">
					{{ formatMessage(messages.signedOutDescription) }}
				</p>
				<Button :disabled="busy" @click="handleSignIn">
					<SpinnerIcon v-if="busy" class="size-4 animate-spin" />
					{{
						busy
							? formatMessage(messages.waiting)
							: formatMessage(messages.signIn)
					}}
				</Button>
			</section>

			<template v-else>
				<p
					v-if="meError"
					class="m-0 rounded-xl border border-solid border-surface-5 bg-bg-raised px-4 py-3 text-sm text-secondary"
				>
					{{ meError }}
				</p>

				<Tabs v-model:value="activeTab" :tabs="tabs" size="large" />

				<section v-if="activeTab === 'announcements'" class="flex flex-col gap-3">
					<div
						v-if="loading.announcements"
						class="flex items-center gap-2 text-sm text-secondary"
					>
						<SpinnerIcon class="size-4 animate-spin" />
						{{ formatMessage(messages.loading) }}
					</div>
					<div
						v-else-if="errors.announcements"
						class="flex flex-col items-start gap-2 rounded-2xl border border-solid border-surface-5 bg-bg-raised p-5"
					>
						<span class="text-sm text-secondary">{{ errors.announcements }}</span>
						<Button size="sm" type="outlined" @click="loadAll">
							{{ formatMessage(messages.retry) }}
						</Button>
					</div>
					<p v-else-if="!announcements.length" class="m-0 text-sm text-secondary">
						{{ formatMessage(messages.emptyAnnouncements) }}
					</p>
					<article
						v-for="(item, index) in announcements"
						:key="String(item.id ?? index)"
						class="rounded-2xl border border-solid border-surface-5 bg-bg-raised p-5"
					>
						<div class="mb-2 flex flex-wrap items-center gap-2">
							<h3 class="m-0 text-base font-bold text-contrast">{{ item.title }}</h3>
							<span
								v-if="item.pinned"
								class="rounded-full bg-highlight-green px-2 py-0.5 text-xs font-semibold text-green"
							>
								{{ formatMessage(messages.pinned) }}
							</span>
						</div>
						<p class="m-0 whitespace-pre-wrap text-sm leading-relaxed text-primary">
							{{ item.body }}
						</p>
						<p
							v-if="formatDate(item.published_at) || item.author"
							class="mt-3 mb-0 text-xs text-secondary"
						>
							{{ formatDate(item.published_at) }}
							<span v-if="item.author">· {{ item.author }}</span>
						</p>

						<div v-if="item.attachments?.length" class="mt-4 flex flex-col gap-2">
							<span class="text-xs font-semibold text-secondary">
								{{ formatMessage(messages.attachmentTitle) }}
							</span>
							<template v-for="att in item.attachments" :key="String(att.id)">
								<img
									v-if="attachmentKind(att) === 'image'"
									:src="attachmentUrl(item, att)"
									:alt="att.name"
									class="max-h-72 w-fit rounded-xl border border-solid border-surface-5 object-contain"
								/>
								<div v-else class="flex flex-col gap-2">
									<div class="flex items-center justify-between gap-3 rounded-xl bg-surface-4 px-3 py-2">
										<span class="flex min-w-0 flex-col">
											<span class="truncate text-sm text-contrast">{{ att.name }}</span>
											<span v-if="att.size" class="text-xs text-secondary">
												{{ formatSize(att.size) }}
											</span>
										</span>
										<span class="flex shrink-0 items-center gap-2">
											<Button
												v-if="attachmentKind(att) !== null"
												size="sm"
												type="outlined"
												@click="togglePreview(item, att)"
											>
												{{
													previewOf(item, att)?.open
														? formatMessage(messages.attachmentHide)
														: formatMessage(messages.attachmentPreview)
												}}
											</Button>
											<Button size="sm" type="quiet" @click="openAttachment(item, att)">
												{{ formatMessage(messages.attachmentOpen) }}
											</Button>
										</span>
									</div>

									<div
										v-if="previewOf(item, att)?.open"
										class="rounded-xl border border-solid border-surface-5 bg-bg p-3"
									>
										<div
											v-if="previewOf(item, att)?.loading"
											class="flex items-center gap-2 text-sm text-secondary"
										>
											<SpinnerIcon class="size-4 animate-spin" />
											{{ formatMessage(messages.attachmentPreviewLoading) }}
										</div>
										<p
											v-else-if="previewOf(item, att)?.error"
											class="m-0 text-sm text-secondary"
										>
											{{ previewOf(item, att)?.error }}
										</p>
										<!-- eslint-disable vue/no-v-html -- renderString 已过 xss 过滤 -->
										<div
											v-else-if="attachmentKind(att) === 'markdown'"
											class="markdown-body max-h-96 overflow-y-auto text-sm leading-relaxed text-primary"
											v-html="renderString(previewOf(item, att)?.text ?? '')"
										></div>
										<!-- eslint-enable vue/no-v-html -->
										<pre
											v-else-if="attachmentKind(att) === 'text'"
											class="m-0 max-h-96 overflow-y-auto whitespace-pre-wrap break-words font-mono text-xs leading-relaxed text-primary"
											>{{ previewOf(item, att)?.text ?? '' }}</pre
										>
										<iframe
											v-else-if="attachmentKind(att) === 'pdf'"
											:src="attachmentUrl(item, att) + '?inline=1'"
											:title="att.name"
											class="h-96 w-full rounded-lg border-0 bg-bg-raised"
										></iframe>
									</div>
								</div>
							</template>
						</div>
					</article>
				</section>

				<section v-else-if="activeTab === 'packs'" class="flex flex-col gap-3">
					<div v-if="loading.packs" class="flex items-center gap-2 text-sm text-secondary">
						<SpinnerIcon class="size-4 animate-spin" />
						{{ formatMessage(messages.loading) }}
					</div>
					<div
						v-else-if="errors.packs"
						class="flex flex-col items-start gap-2 rounded-2xl border border-solid border-surface-5 bg-bg-raised p-5"
					>
						<span class="text-sm text-secondary">{{ errors.packs }}</span>
						<Button size="sm" type="outlined" @click="loadAll">
							{{ formatMessage(messages.retry) }}
						</Button>
					</div>
					<p v-else-if="!packs.length" class="m-0 text-sm text-secondary">
						{{ formatMessage(messages.emptyPacks) }}
					</p>
					<article
						v-for="(pack, index) in packs"
						:key="String(pack.id ?? index)"
						class="flex flex-col gap-3 rounded-2xl border border-solid border-surface-5 bg-bg-raised p-5"
					>
						<div class="flex flex-wrap items-center justify-between gap-3">
							<div class="flex flex-wrap items-center gap-2">
								<h3 class="m-0 text-base font-bold text-contrast">{{ pack.name }}</h3>
								<span
									v-if="pack.mc_version"
									class="rounded-full bg-surface-4 px-2 py-0.5 text-xs text-secondary"
								>
									{{ pack.mc_version }}
								</span>
								<span
									v-if="pack.loader"
									class="rounded-full bg-surface-4 px-2 py-0.5 text-xs text-secondary"
								>
									{{ pack.loader }}
								</span>
							</div>
							<Button size="sm" :disabled="!pack.url" @click="openPack(pack)">
								{{ formatMessage(messages.download) }}
							</Button>
						</div>
						<p v-if="pack.description" class="m-0 text-sm text-primary">
							{{ pack.description }}
						</p>
						<dl class="m-0 flex flex-wrap gap-x-6 gap-y-1 text-xs text-secondary">
							<div v-if="pack.size_bytes" class="flex gap-1">
								<dt class="font-semibold">{{ formatMessage(messages.size) }}</dt>
								<dd class="m-0">{{ formatSize(pack.size_bytes) }}</dd>
							</div>
							<div v-if="pack.sha256" class="flex min-w-0 items-center gap-1">
								<dt class="font-semibold">{{ formatMessage(messages.checksum) }}</dt>
								<dd class="m-0 truncate font-mono" :title="pack.sha256">
									{{ pack.sha256.slice(0, 16) }}…
								</dd>
								<button
									type="button"
									class="cursor-pointer rounded border-none bg-transparent p-0 text-secondary hover:text-contrast"
									:title="formatMessage(messages.checksum)"
									@click="copyText(pack.sha256, `pack-${index}`)"
								>
									<CheckIcon
										v-if="copiedKey === `pack-${index}`"
										class="size-3.5 text-green"
									/>
									<CopyIcon v-else class="size-3.5" />
								</button>
							</div>
						</dl>
					</article>
				</section>

				<section v-else class="flex flex-col gap-3">
					<div v-if="loading.servers" class="flex items-center gap-2 text-sm text-secondary">
						<SpinnerIcon class="size-4 animate-spin" />
						{{ formatMessage(messages.loading) }}
					</div>
					<div
						v-else-if="errors.servers"
						class="flex flex-col items-start gap-2 rounded-2xl border border-solid border-surface-5 bg-bg-raised p-5"
					>
						<span class="text-sm text-secondary">{{ errors.servers }}</span>
						<Button size="sm" type="outlined" @click="loadAll">
							{{ formatMessage(messages.retry) }}
						</Button>
					</div>
					<p v-else-if="!servers.length" class="m-0 text-sm text-secondary">
						{{ formatMessage(messages.emptyServers) }}
					</p>
					<article
						v-for="(server, index) in servers"
						:key="String(server.id ?? index)"
						class="flex flex-wrap items-center justify-between gap-3 rounded-2xl border border-solid border-surface-5 bg-bg-raised p-5"
					>
						<div class="flex min-w-0 flex-col gap-1">
							<div class="flex flex-wrap items-center gap-2">
								<h3 class="m-0 text-base font-bold text-contrast">{{ server.name }}</h3>
								<span
									v-if="server.whitelist_required"
									class="rounded-full bg-surface-4 px-2 py-0.5 text-xs text-secondary"
								>
									{{ formatMessage(messages.whitelist) }}
								</span>
							</div>
							<p class="m-0 font-mono text-sm text-primary">{{ addressOf(server) }}</p>
							<p v-if="server.description" class="m-0 text-xs text-secondary">
								{{ server.description }}
							</p>
						</div>
						<Button
							size="sm"
							type="outlined"
							@click="copyText(addressOf(server), `server-${index}`)"
						>
							<CheckIcon
								v-if="copiedKey === `server-${index}`"
								class="size-4 text-green"
							/>
							<CopyIcon v-else class="size-4" />
							{{
								copiedKey === `server-${index}`
									? formatMessage(messages.copied)
									: formatMessage(messages.copyAddress)
							}}
						</Button>
					</article>
				</section>
			</template>
		</div>
	</div>
</template>
