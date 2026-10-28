# 01: Workspace 骨架 + 项目模型 + 状态机

**What to build:** 用户（或 AI）能通过 CLI `--json` 新建项目和查询项目状态：`project create` 生成项目目录与 `project.json`（原子写）、`project status` 返回状态机当前阶段与各阶段产物就绪情况。项目级文件锁保证并发安全。这是所有后续票的地基：Cargo workspace（cc-core + cc-cli）、错误类型、状态机与指纹的**类型与存储**（stage runner 后续票接入）。

**Blocked by:** None (can start immediately)

**Status:** ready-for-agent

依据：spec「Implementation Decisions」的状态机、指纹模型、并发三节；工具链注意——本仓库已设 rustup override 为 stable-gnu，直接 `cargo build/test` 即可，无需 MSVC。

- [ ] `project create --name x --json` 返回 project_id 与目录，落盘 project.json
- [ ] `project status --project ID --json` 返回当前阶段与缺失项
- [ ] project.json 原子写（tmp → fsync → rename）；并发下第二个进程拿不到锁并得到结构化错误
- [ ] 状态机只允许定义的边，非法迁移报错（单测覆盖全部边）
- [ ] 指纹记录结构可序列化进 project.json（stage/state/input_fingerprint/tool_version/outputs/depends_on）
- [ ] cargo test 全绿（GNU 工具链）
