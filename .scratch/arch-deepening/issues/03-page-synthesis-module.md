# 03: 逐页合成 module：把一页的策略收进一个 interface

**What to build:** 把“一页怎么合成”从 synthesize_pages 里抽成一个独立 module：空页静默、降级链游走、超时重试、幂等跳过、原子落盘、失败隔离都由它负责；synthesize_pages 退化成页循环、阶段迁移和报告组装。用户可观察：合成结果、指纹、skipped/generated/failed 列表、降级事件、警告与断点续跑行为完全不变。

**Blocked by:** None (can start immediately)

**Status:** done

- [x] 存在一个逐页合成 interface：输入一页请求与降级链，返回该页 outcome；synthesize_pages 只做循环、阶段进入/退出和报告。
- [x] 空页与有声页共用同一套落盘/幂等路径；不再有两套分支实现。
- [x] 现有 TTS 测试不改断言即可通过；每页指纹、tool_version、产物路径与哈希不变。
- [x] fake 引擎调用次数、降级粘性、重试与断点续跑行为不变。
- [x] 超时与重试仍由请求层负责，引擎 trait seam（fake/mimo/edge/sapi）保持不变。
