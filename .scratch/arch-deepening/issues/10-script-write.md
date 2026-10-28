# 10: 逐字稿写入批次：script_set/import_notes/split/balance

**What to build:** script_set、script_import_notes、script_split、script_balance 收进命令层；按页合并、页序与段落均分规则只有一个实现。用户可观察：AI 通过 CLI/MCP 改第 7 页讲稿，与用户在 app 里改第 7 页得到完全相同的结果和下一次重跑范围。

**Blocked by:** 05

**Status:** done

- [x] CLI/MCP/app 的 script_set 与 import_notes 返回相同形状并执行相同的按页合并规则。
- [x] PPT 备注导入仍按 sldIdLst 页序，跳过页码占位符；行为不变。
- [x] split/balance 只有一份实现；app 与 MCP 的切分/均分结果一致。
- [x] 现有 script 编辑/导入测试通过；空页仍然合法。
- [x] 跨壳测试覆盖只改部分页与空页。
