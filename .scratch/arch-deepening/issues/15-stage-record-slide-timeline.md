# 15: 阶段记录迁移 slide + timeline

**What to build:** slide_render 与 timeline 的“指纹校验 + 依赖校验 + 产物完整性 + 原子落盘”改用 04 的阶段记录 module；删除两个 runner 各自的哈希/产物检查 helper。用户可观察：课件或音轨在不该跳过时仍然重跑，在该跳过时不重跑，产物的哈希与记录字段不变。

**Blocked by:** 04

**Status:** done

- [x] slide 与 timeline 只有在指纹、依赖、产物完整性全部匹配时才跳过。
- [x] 产物记录的路径/哈希/字节数与 project.json 形状保持不变。
- [x] 两个 runner 的本地 sha256/artifacts helper 删除；workspace 测试通过。
- [x] kill -9 后的续跑与失效传播行为不变。
- [x] 新增测试覆盖“产物被篡改”与“依赖出现/消失”两种情况。
