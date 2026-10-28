---
name: coursecast
description: 把 PPT/PDF 课件、逐字稿和出镜视频做成讲解微课（画中画成片）。当用户说“把这套 PPT 做成课”“生成讲解视频”“做个微课”“用我的声音配音”时使用；通过 CourseCast MCP 工具或 cc-cli 完成。
---

# CourseCast（课影）使用说明

CourseCast 把「课件 + 逐字稿 + 出镜视频」合成一条 MP4 微课。所有状态都落在项目目录的
`project.json` / `script.md` / `work/` 里，文件即真相；重复调用是幂等的。

## 何时使用

- 用户要把一套课件（PDF / PPTX / 图片文件夹）做成带讲解的视频。
- 用户已有逐字稿，或想从 PPT 备注导入讲稿。
- 用户想用自己的声音克隆配音，或用系统兜底音色离线出片。

## 前置条件

1. CourseCast 已安装（桌面版）或 `cc-mcp` / `cc-cli` 可执行。
2. 明确数据目录 `--data-dir`；项目位于 `<data-dir>/projects/<project_id>/`。
3. 声音克隆需要 MiMo API Key 和《声音授权确认》。Key 只通过
   `voice_configure` 的 `api_key` 参数写入系统凭据存储，绝不写进项目文件、命令历史或日志。
4. 删项目、清中间产物都是不可逆操作：必须先得到用户明确同意，再传 `confirm:true`。

## 标准流程

每一步先看返回的 `ok` 和 `next`；失败时读 `error.code` / `error.message`，不要盲目重跑。

1. **找到或新建项目**
   - 续跑：`project_list` → `project_status`，看当前 stage 和 `missing`。
   - 新建：`project_create`，参数 `name`（必填）、`resolution`（默认 `1920x1080`）。
2. **渲染课件**：`slide_render`，参数 `project_id`、`source_path`。
   - `source_path` 可以是项目内相对路径（如 `source/deck.pdf`）或绝对路径；
     相对路径不得逃逸项目目录（否则 `path_escape`）。
   - 返回 `page_count` 和 `slides`；后续逐字稿页数以它为准。
3. **准备逐字稿**（三选一或组合）
   - `script_get` 读回现有页面；缺失脚本返回 `{ok:true, exists:false, pages:[]}`，不是错误。
   - `script_import_notes` 从 PPT 备注导入；可用 `pptx_path` 指定文件。
   - `script_set` 写指定页：`pages:[{"page":1,"text":"第一页讲稿"}, ...]`；未提到的页保持不动。
   - 空页不阻塞流程，合成时按 5 秒静默处理。
4. **配置声音并合成**
   - `voice_configure`：`engine` 必填。`engine=mimo` 走克隆（需要 `ref_audio`、`api_key`、
     `consent:true`）；`engine=edge` 直接用微软预置音色（免 Key、免授权），可加
     `edge_voice=xiaoxiao|yunxi` 选音色。选择会持久化到 project.json 的 `tts`，
     试听和生成都按它构建降级链。`fake`/`sapi` 只作为显式合成引擎，不改变项目偏好。
   - `ref_audio` 入库为 24kHz 单声道 wav；`api_key` 存凭据；`consent:true` 记录
     《声音授权确认》。克隆引擎 `mimo` 未授权会拒绝合成，Edge 不需要授权。
   - `voice_synthesize`：`pages` 省略则全部；`engine` 默认 `fake`（离线演示），
     真实克隆用 `mimo`。失败页返回在 `failed_pages`，可只重跑这些页。
5. **处理出镜视频**
   - `person_matte` 可带 `video_path` 入库并抠像；省略则用已入库的 `source/person.mp4`。
   - `person_status` 查看 `has_video` / `matte_done` 和预览帧路径。
   - 不想抠像时调用 `person_matte_skip`：清掉 alpha，生成时用纯色圆角卡片包住原视频。
6. **版式与成片**
   - `layout_set` 只传要改的键：自由位置用 `x_pct` + `y_pct`（0–100，相对剩余画布空间）
     和 `width_pct`（5–40）；旧项目仍可用 `position`（bottom_right / bottom_left / right_bar），
     但不要与 x/y 混用。`shape` = rounded_rect | circle，`transition` = hard | fade。
   - `render` 会先按时间轴公式重建音轨（幂等），再输出 `output/final.mp4`。
   - 改稿后需要重跑时，先 `artifact_clean`（stage = tts | matte | timeline | render | all，
     必须 `confirm:true`）。

## 项目与清理

- `project_list`：列出所有项目，拿到 `project_id` 再续跑。
- `project_status`：stage、四个输入、`person` 产物详情、缺失项和 next。
- `project_delete`：删除整个项目（不可逆）。只有用户明确说“删掉这个项目”时才调用，
  并传 `confirm:true`。

## 返回契约

- 成功：`{ "ok": true, ...产物路径/统计/next }`。
- 失败：`{ "ok": false, "error": { "code": "...", "message": "..." } }`。
- `next` 是建议的下一步；不要跳过用户尚未确认的破坏性操作。
- 不要根据人类可读日志猜状态；以 JSON 的 `stage`、`missing`、产物路径为准。

## 降级与密钥

- 真实链：MiMo（有 Key）→ edge-tts → Windows SAPI。返回的 `fallbacks` 会说明哪一级
  被跳过以及原因；向用户如实说明实际使用的引擎，不要把兜底音色说成克隆音色。
- `401` / 无 Key：提示用户在「配置声音」一步填入有效 Key；不要静默把克隆任务换成
  其他音色后宣称完成。用户同意降级时才继续。
- Key 只走 `voice_configure.api_key`；不要回显 Key，不要写进项目 JSON、脚本或日志。

## 常见故障

| 现象 | 处理 |
|---|---|
| `page_out_of_range` | 逐字稿页数超过课件页数：先重渲染课件或修正 `script_set` 的页码 |
| `script_empty` | 还没有任何讲稿：先 `script_import_notes` 或 `script_set` |
| 某页空 | 正常，按 5 秒静默处理；不要为了填满而编造讲稿 |
| `consent_required` | 先让用户勾选《声音授权确认》（`voice_configure consent:true`） |
| `person_missing` | 先 `person_matte` 入库视频，或确认用户同意卡片兜底后 `person_matte_skip` |
| `upstream_failed` | 上游阶段失败（例如 TTS 有失败页）：先重跑该阶段，再 `render` |
| `project_locked` | 另一个进程正在写这个项目；等待其结束，不要删除 `.lock` |
| 抠像边缘有破洞 | 换更干净的半身素材重跑；或 `person_matte_skip` 用卡片兜底 |
| 渲染结果没变 | 输入指纹未变是正常的幂等复用；改了版式/讲稿/视频才需要重跑 |

## 没有 MCP 时用 CLI

`cc-cli` 是同一命令层的薄壳，参数用 `--project`、`--json`：

```bash
cc-cli --data-dir data project list --json
cc-cli --data-dir data project status --json --project <id>
cc-cli --data-dir data slide_render --json --project <id> --source source/deck.pdf
cc-cli --data-dir data script_set --json --project <id> --pages '[{"page":1,"text":"..."}]'
cc-cli --data-dir data voice_synthesize --json --project <id> --engine mimo
cc-cli --data-dir data person_status --json --project <id>
cc-cli --data-dir data person_matte --json --project <id>
cc-cli --data-dir data render --json --project <id>
cc-cli --data-dir data project delete --json --project <id> --confirm
```

CLI 与 MCP 走同一份 `cc-core` 实现，返回同一套 JSON 形状；选一个即可，不要混用两套逻辑。
