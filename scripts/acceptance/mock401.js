#!/usr/bin/env node
// 故障注入①：本地 MiMo 兼容端点，对所有请求回 401（票 16）。
// 用法: node mock401.js <port-file>
// 监听 127.0.0.1 的随机端口，把实际端口写进 port-file 供验收脚本读取。
const fs = require("fs");
const http = require("http");

const server = http.createServer((req, res) => {
  req.resume();
  req.on("end", () => {
    res.writeHead(401, { "content-type": "application/json" });
    res.end(JSON.stringify({ error: { message: "invalid api key (acceptance injection)" } }));
  });
});

server.listen(0, "127.0.0.1", () => {
  fs.writeFileSync(process.argv[2], String(server.address().port));
});
