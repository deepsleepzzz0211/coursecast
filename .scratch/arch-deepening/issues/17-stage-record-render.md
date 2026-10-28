# 17: 阶段记录迁移 render

**What to build:** render 的指纹、depends_on 与成片完整性校验改用 04 的阶段记录 module；删除 render 自己的 output_intact/hash helper。用户可观察：同样的 timeline/版式/ffmpeg 下成片不重渲；换版式或换人像后只重渲成片；从 failed 显式重试永不跳过。

**Blocked by:** 04

**Status:** done

- [x] render 只有在 timeline/layout/ffmpeg 指纹、voice_full/person 内容哈希、final.mp4 完整性全部匹配时才跳过。
- [x] 从 failed 显式重试仍不跳过；成功后 failed 记录被清除。
- [x] 本地 helper 删除；现有 render 测试通过；翻页点与时长不变。
- [x] 新增测试覆盖“voice_full 内容变了但时长不变”与“成片被篡改”。
- [x] project.json 的 render 记录字段与 depends_on 键保持不变。
