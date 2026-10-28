import { computed, ref } from 'vue'

/**
 * 全局「提示」收集器：把散落在页面里的黄色警告汇总到标题栏的一个 ⚠️ 里。
 *
 * 用模块级单例而不是 provide/inject：标题栏与页面不在同一棵 provide 树上，
 * 且 inject 在 provider 建立前的调用时序问题已经踩过一次（见 0.1.41 假死事故）。
 */
export interface AppWarning {
	id: string
	title: string
	body: string
}

const warnings = ref<AppWarning[]>([])

export const appWarnings = {
	list: computed(() => warnings.value),
	count: computed(() => warnings.value.length),
	has: computed(() => warnings.value.length > 0),
	set(warning: AppWarning) {
		const index = warnings.value.findIndex((item) => item.id === warning.id)
		if (index === -1) {
			warnings.value = [...warnings.value, warning]
			return
		}
		warnings.value = warnings.value.map((item) => (item.id === warning.id ? warning : item))
	},
	clear(id: string) {
		warnings.value = warnings.value.filter((item) => item.id !== id)
	},
}
