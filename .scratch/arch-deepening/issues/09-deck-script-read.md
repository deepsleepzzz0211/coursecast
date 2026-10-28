# 09: 课件 + 逐字稿读取批次：slide_render/slides_list/script_get

**What to build:** slide_render（PDF 与 PPTX 本机转换）、slides_list、script_get 收进命令层；CLI/MCP/app 共用同一条路径。用户可观察：同一份课件在三个壳里渲染出相同页序与缩略图；PPTX 转换失败时都给出“请先另存为 PDF”的结构化提示。

**Blocked by:** 02, 05

**Status:** done

- [x] CLI/MCP/app 的 slide_render 与 script_get 返回相同形状；slides_list 在需要它的壳可用。
- [x] PPTX 转换失败在三个壳返回同一错误码与提示。
- [x] 现有 slide render/script get 测试通过；页序、缺页与缩略图行为不变。
- [x] 跨壳同构测试覆盖 PDF fixture 与损坏的 pptx。
- [x] 壳内不再各自组装课件/逐字稿读取的 JSON。
