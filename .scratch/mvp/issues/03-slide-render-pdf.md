# 03: PDF 课件渲染

**What to build:** `slide_render` 阶段端到端：`slide_render --project ID --source deck.pdf --json` 把 PDF 渲染成一组有序 1920 宽 PNG（pdfium 动态加载 vendor 里的 dll），数字排序禁止字符串排序，CLI 返回页数 + 每页缩略图路径，指纹（源文件哈希 + 渲染器版本 + 目标宽度）落盘。

**Blocked by:** 01

**Status:** ready-for-agent

注意：worktree 内没有 vendor/（gitignore），需从主检出复制 pdfium.dll；测试 fixture 用代码内构造的最小多页 PDF，不依赖 data/。

- [ ] 最小 3 页 PDF fixture → 3 张 PNG，顺序正确
- [ ] `幻灯片N` 式文件名的数字排序正确（10 不排在 2 前）
- [ ] 29 页真实课件渲染 ≤ 8s（用主检出 `data/pdf-test/deck.pdf` 验证）
- [ ] 图片文件夹输入按数字排序（渲染器不解析，只排序）
- [ ] 渲染指纹落盘；重复调用且指纹未变时跳过（幂等）
