# 16: 端到端验收 + 故障注入

**What to build:** W4 验收：脚本化端到端（干净临时目录、fake 引擎）+ 三类故障注入——①TTS 401（Key 失效）→ 降级提示、主流程不中断；②ffmpeg 渲染中途失败 → 阶段标记 failed、单阶段重试成功；③合成中途 kill -9 → 重进续跑不重做。产出验收记录（实际耗时、体积、翻页误差）进验证报告。

**Blocked by:** 14

**Status:** done

依据：spec 必测场景汇总；PRD §11 W4 验收（"干净机器安装后全流程可用"；真实教师试用为人工环节，不在本票）。

- [x] 三类注入全部按预期恢复，无脏产物、无静默过期
      脚本 `scripts/acceptance.sh`（`ONLY=…`/`KEEP=1` 可单跑与保留现场）：
      ① 401 mock → mimo→edge→sapi 降级，2/2 页出音、timeline/render 不中断；
      ② ffmpeg 渲染垫片写半截后退出 1 → stage=failed(rendering)，同段重试 done、
         failed=null、前序产物哈希不变；
      ③ 合成中途 kill -9 → stage=tts_running，重进 skipped=已完成页、只补剩余页、
         无 .tmp 残留、哈希不变。
      验收抓到并修复两个真缺陷：render 重试成功后旧 failed 未清；ffmpeg concat
      demuxer 缺页/坏图退出 0 导致渲染静默出片（加 -xerror），均有回归测试。
- [x] 10 分钟成片端到端总耗时记录（目标 ≤ 15 分钟，其中 TTS 约 3–4 分钟）
      50 页 → 632.81 s（10 分 32.8 秒）成片，产品阶段合计 264.2 s（4 分 24 秒；
      fake TTS 1.34 s，render 261.0 s），容器/时间轴时长误差 0 ms，翻页误差
      最大 33 ms / 平均 13 ms（30fps 单帧量化）；SAPI 真实计时 144 s 合成
      13,858.7 s 音频（96× 实时），见报告 §5。
- [x] 验收数据追加进 docs/VERIFICATION-2026-10-24.md 或新验收报告
      新建 `docs/ACCEPTANCE-2026-10-25.md`（含环境、耗时表、注入记录、缺陷与限制）。
