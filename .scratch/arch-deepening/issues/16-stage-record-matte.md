# 16: 阶段记录迁移 matte（run + preview）

**What to build:** run_matte 与 run_matte_preview 的指纹校验、视频依赖校验、产物完整性与记录落盘改用 04 的阶段记录 module；删除两个入口里重复的 skip/落盘代码。用户可观察：第 5 步提前预览过的抠像在生成时仍然跳过；换视频后旧 alpha 仍然失效。

**Blocked by:** 04

**Status:** done

- [x] 两个 matte 入口通过同一个 module 校验指纹 + 视频依赖 + 产物完整性。
- [x] 预览与生成共用同一份记录；生成时已预览则跳过。
- [x] 换视频后的旧 alpha 失效与卡片兜底行为不变。
- [x] 两个入口的重复 skip/落盘代码删除；现有 matte stage/inference 测试通过。
- [x] 新增测试覆盖“视频哈希变了但模型没变”和“产物被篡改”。
