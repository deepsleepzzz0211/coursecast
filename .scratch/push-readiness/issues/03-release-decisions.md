# 03: 发布决策：仓库可见性、声纹、邮箱与远端

**What to build:** 在动手清理历史前，明确并记录四项决策：仓库公开还是私有；真人声纹样本是移除还是保留（保留需何种授权）；提交作者邮箱是否重写；远端 URL 与默认分支选 main 还是 mvp。决策写入本票，作为后续清理、历史重写和推送的闸门。

**Blocked by:** None (can start immediately)

**Status:** done

依据：`docs/PRE-PUSH-REVIEW-2026-10-28.md` §2 P0-1/P0-3、§7.2 提交邮箱。

- [x] 仓库可见性：公开
- [x] 真人声纹：移除，不进仓库（05 已清理当前树；历史不公开）
- [x] 提交作者邮箱：使用 `deepsleepzzz0211@users.noreply.github.com`
- [x] 远端 URL 与默认分支：`https://github.com/deepsleepzzz0211/coursecast`，main
- [x] 是否授权执行不可逆的历史重写（07）：不重写，改用 orphan 快照（07 关闭）

## Comments

2026-10-29 决策完成：

- 仓库可见性：**公开**（远端为空仓库，尚未推送）。
- 真人声纹：**移除，不进仓库**（当前树已由 05 清理；历史不公开）。
- 提交邮箱：公开快照使用 GitHub noreply，不暴露个人邮箱。
- 远端：`https://github.com/deepsleepzzz0211/coursecast`，默认分支 main。
- 发布方式：**orphan 干净快照**，不重写本地历史；07 关闭为 superseded。
- 用户补充：本项目本体使用 MIT；安装包不签名、无发行方联系方式。二进制分发合规见 11。
