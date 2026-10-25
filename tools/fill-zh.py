"""补齐 zh-CN 语言包中启动器可见的缺失条目。"""
import io
import json
import sys

APP_ZH = {
    'app.file-download.canceling.message': '正在取消下载…',
    'app.file-download.preparing.message': '正在准备下载…',
    'app.file-download.waiting.message': '正在等待服务器响应…',
    'app.modal.install-to-play.unavailable-description.message':
        '该服务器使用的整合包版本目前无法下载，可能已被取消发布或正在审核中。请稍后再试，或联系服务器管理员。',
    'app.modal.install-to-play.unavailable-header.message': '该服务器的整合包不可用',
    'app.settings.sidebar.label.instance-defaults.message': '实例默认设置',
    'app.settings.sidebar.label.system.message': '系统',
    'app.settings.tabs.java-and-memory.message': 'Java 与内存',
    'app.settings.tabs.launch-hooks.message': '启动钩子',
    'app.settings.tabs.resources.message': '资源',
    'app.settings.tabs.syncing.message': '同步',
    'app.settings.tabs.troubleshooting.message': '疑难解答',
    'app.settings.tabs.window.message': '窗口',
    'app.settings.troubleshooting.clear-cache.message': '清除缓存',
    'app.settings.troubleshooting.clear-cache.confirm.title.message': '确定清除应用缓存？',
    'app.settings.troubleshooting.report-ad.copied.message': '已复制',
    'app.settings.troubleshooting.report-ad.copy.message': '复制广告信息',
    'app.settings.troubleshooting.report-ad.description.message':
        '看到不当或令人反感的广告？在广告显示时复制其信息，并发送给 <support>Modrinth 支持团队</support>。我们会审核并移除该广告。',
    'app.settings.troubleshooting.report-ad.no-ad.text.message': '请等待广告加载完成后再试。',
    'app.settings.troubleshooting.report-ad.no-ad.title.message': '未找到广告',
    'app.settings.troubleshooting.report-ad.title.message': '举报广告',
    'app.shared-instance-publish.preparing.message': '正在准备更新…',
    'app.shared-instance-publish.pushed.message': '更新已推送',
    'app.shared-instance-publish.uploading.message': '正在推送更新…',
    'instance.settings.defaults.open-app-settings.message': '管理默认设置',
    'instance.settings.sidebar.label.game.message': '游戏',
    'instance.settings.sidebar.label.instance.message': '实例',
    'instance.settings.tabs.hooks.default-settings-description.message': '除非在此处覆盖，否则该实例使用默认的启动钩子。',
    'instance.settings.tabs.hooks.label.message': '启动钩子',
    'instance.settings.tabs.java.default-settings-description.message': '除非在此处覆盖，否则该实例使用默认的 Java 与内存设置。',
    'instance.settings.tabs.java.label.message': 'Java 与内存',
    'instance.settings.tabs.syncing.message': '同步',
    'instance.settings.tabs.window.default-settings-description.message': '除非在此处覆盖，否则该实例使用默认的窗口设置。',
    'instance.settings.tabs.window.label.message': '窗口',
    'instance.shared-instance.file-not-allowed.title.message': '不允许的文件类型',
    'instance.shared-instance.publish-review.changes-description.message': '查看自上次更新以来检测到的改动，推送时会自动包含这些改动。',
    'instance.shared-instance.publish-review.changes-tab.message': '改动',
    'instance.shared-instance.publish-review.file-type-not-allowed.message': '此文件类型不允许推送。',
    'instance.shared-instance.publish-review.files-description.message': '除检测到的改动外，你也可以额外选择单个文件一并发布。',
    'instance.shared-instance.publish-review.files-tab.message': '文件',
    'instance.shared-instance.publish-review.storage-over-limit.message': '超出配额 {size}，请取消选择部分文件后再推送。',
    'instance.shared-instance.publish-review.storage-title.message': '存储配额',
    'instance.shared-instance.publish-review.storage-usage.message': '{used} / {limit}',
}

UI_ZH = {
    'changelog.product.hosting.defaultMessage': '托管',
    'hosting.save-banner.saved.defaultMessage': '已保存',
    'external-files.permissions-card.file-database-id.defaultMessage': '数据库 ID',
    'color-picker.hex-value.defaultMessage': '{label} 十六进制色值',
    'color-picker.hue.defaultMessage': '{label} 色相',
    'color-picker.saturation-and-brightness.defaultMessage': '{label} 饱和度与亮度',
    'servers.region.out-of-stock.defaultMessage': '（缺货）',
    'servers.region.ping.failed.defaultMessage': '延迟测试失败',
    'servers.region.ping.latency.defaultMessage': '{ping}ms',
    'servers.region.ping.testing.defaultMessage': '正在测试连接…',
    'servers.backups.create.in-progress.defaultMessage': '已有备份在排队或正在进行中',
    'servers.backups.create.slots-full.defaultMessage': '全部 {limit} 个手动备份槽位都在使用中',
    'servers.backups.delete.error.defaultMessage': '删除备份失败（共 {count} 个）',
    'servers.backups.history.auto-badge.defaultMessage': '自动',
    'servers.backups.history.column.created.defaultMessage': '创建时间',
    'servers.backups.history.column.created-by.defaultMessage': '创建者',
    'servers.backups.history.column.name.defaultMessage': '名称',
    'servers.backups.history.creator-avatar-alt.defaultMessage': '{username} 的头像',
    'servers.backups.history.creator.support.defaultMessage': '支持',
    'servers.backups.history.more-options.defaultMessage': '{name} 的更多选项',
    'servers.backups.history.no-results.defaultMessage': '没有匹配的备份。',
    'servers.backups.history.rename.defaultMessage': '重命名',
    'servers.backups.history.restore.defaultMessage': '恢复',
    'servers.backups.history.search-label.defaultMessage': '搜索备份',
    'servers.backups.history.search-placeholder.defaultMessage': '搜索备份…',
    'servers.backups.history.title.defaultMessage': '备份历史',
    'servers.backups.interval.days.defaultMessage': '{days} 天',
    'servers.backups.interval.every-days.defaultMessage': '每 {days} 天',
    'servers.backups.interval.every-hours.defaultMessage': '每 {hours} 小时',
    'servers.backups.interval.hours.defaultMessage': '{hours} 小时',
    'servers.backups.interval.weekly.defaultMessage': '每周',
    'servers.backups.load-error.description.defaultMessage': '无法加载该服务器的备份。',
    'servers.backups.load-error.title.defaultMessage': '备份加载失败',
    'servers.backups.restore.operation-in-progress.defaultMessage': '已有备份操作在排队或正在进行中',
    'servers.backups.restore.server-running.defaultMessage': '请先停止服务器再恢复备份',
    'servers.backups.rotation-modal.admonition-body.defaultMessage': '减少自动备份槽位意味着最旧的自动备份将被删除。',
    'servers.backups.rotation-modal.admonition-header.defaultMessage': '删除警告',
    'servers.backups.rotation-modal.backups-label.defaultMessage': '{count} 个备份',
    'servers.backups.rotation-modal.confirm.defaultMessage': '保存并删除',
    'servers.backups.rotation-modal.header.defaultMessage': '移除自动备份？',
    'servers.backups.slots.auto.next.defaultMessage': '下次 {time}',
    'servers.backups.slots.auto.next-soon.defaultMessage': '即将开始下次备份',
    'servers.backups.slots.auto.off.defaultMessage': '关闭',
    'servers.backups.slots.auto.off-description.defaultMessage': '在设置中开启自动备份',
    'servers.backups.slots.auto.title.defaultMessage': '自动备份',
    'servers.backups.slots.limit.defaultMessage': '/ {limit} 个槽位',
    'servers.backups.slots.manual.description.defaultMessage': '手动创建',
    'servers.backups.slots.manual.title.defaultMessage': '手动备份',
    'servers.backups.toolbar.more-create-options.defaultMessage': '更多备份选项',
    'servers.settings.backups.auto.description.defaultMessage': '按固定间隔自动备份服务器。',
    'servers.settings.backups.auto.title.defaultMessage': '自动备份',
    'servers.settings.backups.interval.title.defaultMessage': '备份间隔',
    'servers.settings.backups.manual-slots.description.defaultMessage': '选择在 {limit} 个备份槽位中，有多少用于手动备份、多少用于自动备份。',
    'servers.settings.backups.manual-slots.title.defaultMessage': '手动备份槽位',
    'servers.settings.backups.save.error-title.defaultMessage': '更新备份设置失败',
}


def set_path(tree, dotted, value):
    parts = dotted.split('.')
    node = tree
    for part in parts[:-1]:
        nxt = node.get(part)
        if not isinstance(nxt, dict):
            nxt = {}
            node[part] = nxt
        node = nxt
    node[parts[-1]] = value


def fill(zh_path, en_path, mapping):
    zh = json.load(io.open(zh_path, encoding='utf-8'))
    en = json.load(io.open(en_path, encoding='utf-8'))

    def flat(o, p=''):
        r = {}
        for k, v in o.items():
            kp = f'{p}.{k}' if p else k
            if isinstance(v, dict):
                r.update(flat(v, kp))
            else:
                r[kp] = v
        return r

    en_flat = flat(en)
    added = 0
    unknown = []
    for key, val in mapping.items():
        if key not in en_flat:
            unknown.append(key)
            continue
        set_path(zh, key, val)
        added += 1
    with io.open(zh_path, 'w', encoding='utf-8', newline='\n') as f:
        json.dump(zh, f, ensure_ascii=False, indent='\t')
        f.write('\n')
    return added, unknown


if __name__ == '__main__':
    a = fill('apps/app-frontend/src/locales/zh-CN/index.json',
             'apps/app-frontend/src/locales/en-US/index.json', APP_ZH)
    u = fill('packages/ui/src/locales/zh-CN/index.json',
             'packages/ui/src/locales/en-US/index.json', UI_ZH)
    print('app added', a[0], 'unknown', a[1])
    print('ui  added', u[0], 'unknown', u[1])
