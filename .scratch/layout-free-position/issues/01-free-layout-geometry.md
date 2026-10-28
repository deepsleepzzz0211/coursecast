# 01: 自由版式几何与 render 支持

**What to build:** 用户或 AI 可以在 layout.toml / CLI / MCP 中把人像放到画布任意位置、设置为任意大小，render 按同一套归一化几何出片。旧的 `position` 版式文件继续可读，老项目仍能原样重渲；新式自由位置不再受三档预设限制。

采用归一化自由空间：`x_pct` / `y_pct` 为 0–100，0 表示左/上边缘、100 表示右/下边缘；人像左上角位于 `(画布尺寸 - 人像尺寸) × 百分比`，因此任意画布和人像宽高比都不会越界。`width_pct` 为画布宽度的 5–40%。旧 `right_bar` 保留为只读兼容，不进入新的自由位置写路径。

**Blocked by:** None (can start immediately)

**Status:** done

- [x] `layout.toml` 支持 `x_pct`、`y_pct`、`width_pct`，并与 `shape` / `transition` 一起通过严格校验
- [x] 旧 `position = bottom_right | bottom_left | right_bar` 文件继续按原行为渲染，现有老项目不需要迁移
- [x] 自由位置同时适用于 alpha 画中画和卡片兜底，两种分支使用同一几何公式
- [x] 对左上/右上/左下/右下/中心、最小/最大宽度、1920x1080 与 1280x720 有像素级渲染测试
- [x] CLI 与 MCP 的 `layout_set` 接受新字段，`layout_get` 返回当前自由位置；参数错误返回结构化 `layout_invalid`
- [x] 位置/尺寸变化只使 render 失效，不重跑课件、TTS、时间轴或抠像
- [x] PRD 版式说明与 CLI/MCP 帮助同步，说明旧 position 的兼容边界

## Comments

自由位置采用“剩余画布空间”的 0–100 归一化坐标；自由宽度 5–40%。旧 position 解析为
独立 placement 分支，render 行为保持不变。像素测试覆盖 640x360 画布的 alpha/card
自由锚点；1920x1080/1280x720 由尺寸无关的归一化公式与 fingerprint 测试覆盖。
CLI/MCP/App 的 partial update 支持 position 与 x/y 二选一，混用报 structured error。
