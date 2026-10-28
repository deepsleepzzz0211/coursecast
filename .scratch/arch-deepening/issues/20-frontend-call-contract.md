# 20: 前端 call() 结果契约：Tauri 与 mock 归一

**What to build:** 前端 `call()` 的两个 adapter（Tauri 与浏览器 mock）返回同一个结果/错误契约：成功是 `{ok:true, ...}`，失败是 `{ok:false, error:{code,message}}`，不再一边 resolve、一边 throw。组件只写一条错误路径。用户可观察：同一个失败（例如未授权）在真实壳和浏览器预览里表现一致。

**Blocked by:** 05

**Status:** done

- [x] Tauri 与 mock adapter 返回相同成功/失败形状；调用点不再需要同时写 `if (v.ok)` 和 `try/catch`。
- [x] 组件不再按 `inTauri` 分支处理错误。
- [x] TypeScript 类型反映统一契约；`vite build` 与 `vue-tsc` 通过。
- [x] 浏览器走查覆盖 建项目 → 课件 → 逐字稿 → 声音 → 出镜视频 → 生成。
- [x] mock 与真实路径对同一个失败场景给出相同错误码与文案结构。
