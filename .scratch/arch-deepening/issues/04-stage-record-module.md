# 04: 阶段记录 module：输入指纹协议只有一个实现（expand）

**What to build:** 新增一个阶段记录 module，负责校验“指纹匹配 + depends_on 不陈旧 + 产物完整”，并负责产物哈希与记录的原子落盘。先只加 module 和测试，不改现有 runner。它把“旧产物不得被当成新产物”这条不变量变成一个可复用的 interface。

**Blocked by:** None (can start immediately)

**Status:** done

- [x] module 提供小 interface：verify（指纹/依赖/产物）、hash artifact、land record（原子写 + 保存）。
- [x] 单元测试覆盖指纹不符、缺依赖、依赖陈旧、产物缺失、产物被篡改、成功跳过、原子写。
- [x] 现有 stage runner 不改；workspace 测试全绿。
- [x] 没有生产调用方；module 已可被后续迁移直接使用。
- [x] 记录与 project.json 的现有字段/形状保持兼容。
