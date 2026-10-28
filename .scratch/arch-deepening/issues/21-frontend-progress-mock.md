# 21: 前端进度 store + 步骤门控 + fixture mock

**What to build:** 前端只保留一个进度 store，消费 13 的 observer 事件；步骤门控（页数、授权、出镜视频）收进一个地方，导航与按钮共用；浏览器 mock 改成回放 fixture 结果/事件的 adapter，不再手写第二套阶段、失效与授权状态机。用户可观察：第 5/6 步的进度与门控一致，mock 预览不再和真实路径说不同的话。

**Blocked by:** 13, 20

**Status:** done

- [x] Step5 与 Step6 共用一个进度 store；不再有两套 mini 状态机各自解释事件。
- [x] 页数、授权、出镜视频的步骤门控只有一处计算；导航与操作按钮都使用它。
- [x] mock 只回放 fixture 的结果与事件，不实现自己的阶段/失效/授权逻辑。
- [x] mock 与真实契约在成功和失败路径上一致；浏览器全流程走查通过。
- [x] 进度监听失败或事件丢失不阻塞门控与生成（restore first 行为保留）。

## Comments

2026-10-26 验收：progress store、单一门槛、fixture 素材均已完成，浏览器 mock 全流程走查通过。
mock 已隔离为 `web/src/mockAdapter.ts` 的 dev-only adapter，静态素材与预置进度事件来自
`mockFixtures.ts`。2026-10-26 代码整洁收尾：移除 localStorage 与阶段/失效/授权状态机，
每个命令固定回放 fixture 结果，`generate` / `person_matte` 回放预置事件序列；
仅保留文件名触发的错误 fixture（不依赖历史状态），第三项已满足。
