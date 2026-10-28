# 19: 阶段记录契约：删除重复 helper + 跨 stage 不变量测试

**What to build:** 所有 stage runner 迁移完成后，删除剩余的 sha256/artifacts/hash 重复 helper，确保只有阶段记录 module 实现这套协议；补一组跨 stage 的不变量测试，证明五个 stage 对“指纹陈旧、依赖陈旧、产物缺失、产物被篡改、原子落盘、显式重试”给出同一套结论。

**Blocked by:** 15, 16, 17, 18

**Status:** done

- [x] 没有任何 stage module 再定义自己的 sha256/artifacts/hash helper；全部走阶段记录 module。
- [x] 跨 stage 不变量测试覆盖上述矩阵，每个 stage 行为一致。
- [x] depends_on 的键方言被文档化并由 module 统一校验；不再有 ad-hoc staleness 检查。
- [x] 现有全部测试通过；失效传播与断点续跑行为不变。
- [x] 新 stage 的接入只需要提供指纹/依赖/产物，不再复制协议代码。
