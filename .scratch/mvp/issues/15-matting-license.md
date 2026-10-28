# 15: 抠像许可路线落地

**What to build:** 按 W1 三选一的**用户决策**执行：A. 产品整体开源（GPL-3.0，加 LICENSE 与声明）；B. 主选 Apache-2.0 的 MODNet 并实现时域中值平滑补边缘抖动（RVM 降级为自用版可选组件）；C. 向 RVM 作者申请商业授权（成本未知）。本票改变抠像模块实现与流水线时间预算（RVM 8s → MODNet 约 2.2 分钟/30s 素材）。

**Blocked by:** 09

**Status:** done

依据：验证报告 §8（RVM 确认 GPL-3.0，权重同许可）；spec Further Notes W1 未决项。**需要用户拍板后才能动工。**

- [x] 决策记录（ADR 或验证报告追加节：docs/adr/0001）
- [x] 按决策完成对应实现/法律动作（A 路线：LICENSE + README 声明；MODNet 实装不适用）
- [x] 若走 B：…（未走 B，不适用；时间预算维持 PRD §9 现值）
