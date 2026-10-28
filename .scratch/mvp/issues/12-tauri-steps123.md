# 12: Tauri 壳 + 前端（Step1–3）

**What to build:** cc-app：Tauri 2 桌面壳 + Vue 3 + Element Plus，六步向导的前三步——①新建项目（名称 + 分辨率）②传课件（拖 PDF 直用；pptx 尝试本机转换、失败提示另存为 PDF；缩略图网格）③逐字稿（整段粘贴分隔符切分 / 逐页填写 / PPT 备注导入；页数段落不符时"自动均分 + 拖拽调整"；空页标黄按 5s 静默）。Tauri command 直接调 cc-core（与 CLI 同源，不另起一路逻辑）。重启应用进度不丢。

**Blocked by:** 10

**Status:** ready-for-agent

依据：PRD §4.2 Step1–3、§6.4 前端栈（无路由库，currentStep 状态机）。本票起需要 MSVC 工具链（VS Build Tools 安装中），若未就绪此票等待。

- [ ] 三步向导可走通：建项目 → 拖 PDF 出缩略图 → 填逐字稿
- [ ] pptx 失败路径：无 Office 环境下给出"请先另存为 PDF"提示
- [ ] 页数段落不匹配 → 自动均分 + 拖拽调整界面可用
- [ ] 空页标黄，不阻塞流程
- [ ] 重启应用项目与进度还在
