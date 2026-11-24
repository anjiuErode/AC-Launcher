import { Channel, invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { reactive, ref } from 'vue'

export interface ExternalFileDownloadProgress {
	stage: 'waiting' | 'downloading' | 'saving'
	downloadedBytes: number
	totalBytes: number | null
}

export interface ExternalFileDownloadTask {
	id: string
	filename: string
	serverId?: string
	serverName?: string
	status: 'running' | 'succeeded' | 'failed' | 'canceled'
	stage: 'preparing' | ExternalFileDownloadProgress['stage']
	downloadedBytes: number
	totalBytes: number | null
	rate: number
	lastRead: number
	createdAt: string
	finishedAt?: string
	error?: string
	paused: boolean
	canceling: boolean
	cancel: () => Promise<void>
	togglePause?: () => Promise<void>
	retry?: () => Promise<void>
	dismiss?: () => Promise<void>
}

export const externalFileDownloads = ref(new Map<string, ExternalFileDownloadTask>())

interface UrlDownloadSnapshot extends ExternalFileDownloadProgress {
	id: string
	filename: string
	status: ExternalFileDownloadTask['status'] | 'paused' | 'publishing'
	createdAt: string
	finishedAt: string | null
	error: string | null
	revision: number
}

const snapshotRevisions = new Map<string, number>()
let listenForSnapshots: Promise<unknown> | undefined
let loadDownloads: Promise<void> | undefined

export function loadExternalUrlDownloads() {
	listenForSnapshots ??= listen<UrlDownloadSnapshot>('external-url-download', ({ payload }) =>
		applySnapshot(payload),
	).catch((error) => {
		listenForSnapshots = undefined
		throw error
	})
	loadDownloads ??= listenForSnapshots
		.then(async () => {
			const snapshots = await invoke<UrlDownloadSnapshot[]>('plugin:files|files_list_url_downloads')
			for (const snapshot of snapshots) applySnapshot(snapshot)
		})
		.catch((error) => {
			loadDownloads = undefined
			throw error
		})
	return loadDownloads
}

function applySnapshot(snapshot: UrlDownloadSnapshot) {
	const { id, stage, downloadedBytes, totalBytes } = snapshot
	if (snapshot.revision <= (snapshotRevisions.get(id) ?? -1)) return
	snapshotRevisions.set(id, snapshot.revision)

	let task = externalFileDownloads.value.get(id)
	if (!task) {
		const newTask = reactive<ExternalFileDownloadTask>({
			id,
			filename: snapshot.filename,
			status: 'running',
			stage: 'preparing',
			downloadedBytes: 0,
			totalBytes: null,
			rate: 0,
			lastRead: performance.now(),
			createdAt: snapshot.createdAt,
			paused: false,
			canceling: false,
			async cancel() {
				newTask.canceling = true
				try {
					await controlDownload(id, 'cancel')
				} finally {
					newTask.canceling = false
				}
			},
			togglePause: async () => {
				if (newTask.paused) await runDownload(id)
				else await controlDownload(id, 'pause')
			},
			retry: () => runDownload(id),
			dismiss: () => controlDownload(id, 'dismiss'),
		})
		externalFileDownloads.value.set(id, newTask)
		task = newTask
	}

	const now = performance.now()
	const paused = snapshot.status === 'paused'
	if (
		snapshot.status !== 'running' ||
		stage !== 'downloading' ||
		task.stage !== 'downloading' ||
		task.status !== 'running' ||
		task.paused ||
		downloadedBytes < task.downloadedBytes
	) {
		task.rate = 0
		task.lastRead = now
	} else if (downloadedBytes > task.downloadedBytes) {
		const elapsed = Math.max(1, now - task.lastRead)
		const rate = ((downloadedBytes - task.downloadedBytes) * 1000) / elapsed
		task.rate = task.rate && elapsed < 5000 ? task.rate * 0.6 + rate * 0.4 : rate
		task.lastRead = now
	}

	task.status =
		snapshot.status === 'paused' || snapshot.status === 'publishing' ? 'running' : snapshot.status
	task.paused = paused
	task.stage = stage
	task.downloadedBytes = downloadedBytes
	task.totalBytes = totalBytes
	task.finishedAt = snapshot.finishedAt ?? undefined
	task.error = snapshot.error ?? undefined
}

async function runDownload(id: string) {
	const snapshot = await invoke<UrlDownloadSnapshot>('plugin:files|files_start_url_download', {
		id,
	})
	applySnapshot(snapshot)
}

async function controlDownload(id: string, action: 'pause' | 'cancel' | 'dismiss') {
	const snapshot = await invoke<UrlDownloadSnapshot | null>(
		'plugin:files|files_control_url_download',
		{ id, action },
	)
	if (snapshot) {
		applySnapshot(snapshot)
	} else {
		snapshotRevisions.set(id, Infinity)
		externalFileDownloads.value.delete(id)
	}
}

function getExternalFilename(url: URL, suggestedFilename?: string) {
	try {
		const basename = url.pathname.split('/').pop() ?? ''
		const filename = (suggestedFilename ?? decodeURIComponent(basename)).trim()
		if (
			filename &&
			filename.length <= 255 &&
			!/[<>:"/\\|?*\p{Cc}]/u.test(filename) &&
			!filename.endsWith('.') &&
			!/^(?:con|prn|aux|nul|com[1-9¹²³]|lpt[1-9¹²³])(?:\.|$)/i.test(filename)
		) {
			return filename
		}
	} catch {
		// Keep the fallback when the URL contains an invalid percent-encoded name.
	}

	return 'download'
}

export async function startManualUrlDownload(url: URL) {
	await loadExternalUrlDownloads()
	const snapshot = await invoke<UrlDownloadSnapshot | null>(
		'plugin:files|files_create_url_download',
		{
			url: url.href,
			filename: getExternalFilename(url),
		},
	)
	if (!snapshot) return false
	applySnapshot(snapshot)
	return true
}

export async function startExternalUrlDownload(
	url: URL,
	failureMessage: string,
	suggestedFilename?: string,
) {
	if ((url.protocol !== 'http:' && url.protocol !== 'https:') || url.username || url.password) {
		throw new Error('Invalid download URL')
	}

	const filename = getExternalFilename(url, suggestedFilename)
	const saveId = await invoke<string | null>('plugin:files|files_select_external', {
		filename,
		filter: null,
	})
	if (!saveId) return false

	void saveExternalUrl(url, saveId, filename, failureMessage)
	return true
}

async function saveExternalUrl(url: URL, saveId: string, filename: string, failureMessage: string) {
	let releaseRequest: Promise<void> | undefined
	function release() {
		releaseRequest ??= invoke<void>('plugin:files|files_release_external', { saveId }).catch(
			(error) => {
				releaseRequest = undefined
				throw error
			},
		)
		return releaseRequest
	}

	const task = trackExternalFileDownload(saveId, filename, release)
	try {
		const onProgress = new Channel<ExternalFileDownloadProgress>(task.update)
		const savedFilename = await invoke<string>('plugin:files|files_save_external', {
			saveId,
			request: { url: url.href },
			onProgress,
		})
		task.succeed(savedFilename)
	} catch {
		const canceled = await releaseRequest?.then(
			() => true,
			() => false,
		)
		if (canceled) task.canceled()
		else task.fail(failureMessage)
	} finally {
		await release().catch(() => undefined)
	}
}

export function trackExternalFileDownload(
	id: string,
	filename: string,
	onCancel: () => Promise<void>,
	server: { serverId?: string; serverName?: string } = {},
) {
	const task = reactive<ExternalFileDownloadTask>({
		id,
		filename,
		...server,
		status: 'running',
		stage: 'preparing',
		downloadedBytes: 0,
		totalBytes: null,
		rate: 0,
		lastRead: performance.now(),
		createdAt: new Date().toISOString(),
		paused: false,
		canceling: false,
		async cancel() {
			if (task.status !== 'running' || task.canceling) return
			task.canceling = true
			try {
				await onCancel()
				if (task.status === 'running') finish('canceled')
			} finally {
				task.canceling = false
			}
		},
	})
	externalFileDownloads.value.set(id, task)
	let previousBytes = 0
	let previousTime = performance.now()

	function finish(status: ExternalFileDownloadTask['status']) {
		task.status = status
		task.finishedAt = new Date().toISOString()
		task.rate = 0
	}

	return {
		update(progress: ExternalFileDownloadProgress) {
			if (task.status !== 'running' || task.canceling) return
			const now = performance.now()
			if (progress.stage !== 'downloading' || progress.downloadedBytes < previousBytes) {
				task.rate = 0
			} else if (progress.downloadedBytes > previousBytes) {
				const rate =
					((progress.downloadedBytes - previousBytes) * 1000) / Math.max(1, now - previousTime)
				task.rate = task.rate ? task.rate * 0.6 + rate * 0.4 : rate
				task.lastRead = now
			}
			task.stage = progress.stage
			task.downloadedBytes = progress.downloadedBytes
			task.totalBytes = progress.totalBytes
			previousBytes = progress.downloadedBytes
			previousTime = now
		},
		succeed(filename: string) {
			task.filename = filename
			finish('succeeded')
		},
		fail(error: string) {
			task.error = error
			finish('failed')
		},
		canceled() {
			finish('canceled')
		},
	}
}

export function getExternalFileDownloadRate(task: ExternalFileDownloadTask, now: number) {
	return task.status === 'running' &&
		task.stage === 'downloading' &&
		!task.paused &&
		!task.canceling &&
		now - task.lastRead < 5000
		? task.rate
		: 0
}
