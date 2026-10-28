# 02: 锁所有权统一：runner 不锁，命令层锁

**What to build:** 项目锁只在一个地方获取：命令边界（当前是壳，命令层落地后是命令层）。slide_render 与 voice_synthesize 不再在 runner 内部加锁；所有 mutating 命令在调用 runner 前获取一次独占锁，只读状态用共享锁。用户可观察：并发写仍然被拒绝，崩溃后锁自动释放，断点续跑不受影响。

**Blocked by:** None (can start immediately)

**Status:** done

- [x] slide_render 与 voice_synthesize 的 runner 不再内部获取项目锁；调用方获取。
- [x] 每个 mutating 命令路径只获取一次独占锁；project status 仍用共享锁。
- [x] 第二个并发写进程仍被拒绝；kill -9 后重进仍能续跑；锁释放语义不变。
- [x] 测试覆盖独占/共享锁与并发拒绝；workspace 全绿。
- [x] 规则写进相关 module 的文档注释，后续命令层直接复用。
