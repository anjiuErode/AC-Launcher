import { useQuery, useQueryClient } from '@tanstack/vue-query'
import { invoke } from '@tauri-apps/api/core'
import { onScopeDispose, watch } from 'vue'

import { useAppEvent } from '@/composables/use-app-event'
import { get_many as getInstances } from '@/helpers/instance'
import { get_all as getRunningProcesses } from '@/helpers/process'
import type { AppEvents } from '@/providers/app-events'

interface RunningProcess {
	instance_id: string
}

interface PresenceState {
	running: boolean
	instanceName: string
	mcVersion: string
	loader: string
}

function samePresence(previous: PresenceState | undefined, next: PresenceState): boolean {
	return (
		previous !== undefined &&
		previous.running === next.running &&
		previous.instanceName === next.instanceName &&
		previous.mcVersion === next.mcVersion &&
		previous.loader === next.loader
	)
}

const runningKey = ['guild-presence-running'] as const

/**
 * 把「现在在玩哪个实例」同步给 Rust。
 *
 * 心跳本身在 Rust 侧（30 秒一次），这里只负责在游戏启停、实例切换时
 * 更新那份状态；界面最小化后不再参与，也不会影响心跳。
 */
export function useGuildPresenceSync(events: AppEvents) {
	const queryClient = useQueryClient()
	let revision = 0
	let write = Promise.resolve()
	let lastWritten: PresenceState | undefined
	let reportRequested = false

	const { data } = useQuery({
		queryKey: runningKey,
		queryFn: async ({ signal }) => {
			const snapshotRevision = revision
			const processes = (await getRunningProcesses()) as RunningProcess[]
			signal.throwIfAborted()
			const ids = [...new Set(processes.map((process) => process.instance_id))]
			const instances = ids.length ? await getInstances(ids) : []
			const instancesById = new Map(instances.map((instance) => [instance.id, instance]))
			const instance = ids.map((id) => instancesById.get(id)).find((item) => item !== undefined)
			return {
				revision: snapshotRevision,
				state: {
					running: instance !== undefined,
					instanceName: instance?.name ?? '',
					mcVersion: instance?.game_version ?? '',
					loader: instance?.loader ?? '',
				},
			}
		},
	})

	// 必须显式传入事件总线：App.vue 里 provider 建立之前 inject 会抛
	// "Injection Symbol(modrinth:appEvents) not found"，导致整个 setup 失败
	useAppEvent(
		'process',
		() => {
			const nextRevision = ++revision
			void queryClient.cancelQueries({ queryKey: runningKey }).then(() => {
				if (nextRevision === revision) {
					return queryClient.invalidateQueries({ queryKey: runningKey })
				}
			})
		},
		events,
	)

	watch(
		data,
		(value) => {
			if (!value) return
			write = write
				.then(async () => {
					if (data.value !== value || value.revision !== revision) return
					if (!samePresence(lastWritten, value.state)) {
						await invoke('guild_set_presence_state', value.state)
						lastWritten = value.state
						reportRequested = false
					}
					// 写入期间若有新事件，下一次写入会决定是否立即上报。
					if (
						data.value === value &&
						value.revision === revision &&
						value.state.running &&
						!reportRequested
					) {
						reportRequested = true
						void invoke('guild_report_presence').catch(() => {})
					}
				})
				.catch(() => {
					// 状态同步失败不影响游戏。
				})
		},
		{ immediate: true },
	)

	onScopeDispose(() => {
		revision++
	})
}
