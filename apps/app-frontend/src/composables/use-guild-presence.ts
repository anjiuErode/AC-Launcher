import { useQuery, useQueryClient } from '@tanstack/vue-query'
import { invoke } from '@tauri-apps/api/core'
import { watch } from 'vue'

import { useAppEvent } from '@/composables/use-app-event'
import { get_many as getInstances } from '@/helpers/instance'
import { get_all as getRunningProcesses } from '@/helpers/process'

interface RunningProcess {
	uuid: string
	instance_id: string
}

interface RunningInstance {
	id: string
	name: string
	game_version?: string
	loader?: string
}

const runningKey = ['guild-presence-running'] as const

/**
 * 把「现在在玩哪个实例」同步给 Rust。
 *
 * 心跳本身在 Rust 侧（30 秒一次），这里只负责在游戏启停、实例切换时
 * 更新那份状态；界面最小化后不再参与，也不会影响心跳。
 */
export function useGuildPresenceSync() {
	const queryClient = useQueryClient()

	const { data } = useQuery({
		queryKey: runningKey,
		queryFn: async () => {
			const processes =
				((await getRunningProcesses().catch(() => [])) ?? []) as RunningProcess[]
			const ids = processes.map((process) => process.instance_id)
			const instances = (ids.length ? await getInstances(ids).catch(() => []) : []) as
				| RunningInstance[]
				| undefined
			const first = processes[0]
			const instance = first ? instances?.find((item) => item.id === first.instance_id) : undefined
			return {
				running: Boolean(first && instance),
				instanceName: instance?.name ?? '',
				mcVersion: instance?.game_version ?? '',
				loader: instance?.loader ?? '',
			}
		},
	})

	useAppEvent('process', () => {
		void queryClient.invalidateQueries({ queryKey: runningKey })
	})

	watch(
		data,
		(value) => {
			if (!value) return
			void invoke('guild_set_presence_state', {
				instanceName: value.instanceName,
				mcVersion: value.mcVersion,
				loader: value.loader,
				running: value.running,
			}).catch(() => {
				// 状态推送失败不影响游戏，静默即可
			})
			// 刚进游戏时立刻报一次，不必等下一个 30 秒
			if (value.running) {
				void invoke('guild_report_presence').catch(() => {})
			}
		},
		{ immediate: true },
	)
}
