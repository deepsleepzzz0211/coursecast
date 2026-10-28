#!/usr/bin/env node
// 验收脚本的 JSON 读取/断言小工具（票 16）——只做受限查询，不执行动态代码。
// 用法:
//   node json.js get <file> <path>                        # 点分路径，如 fallbacks.0.from
//   node json.js assert <file> <path> <json值>             # 严格相等（JSON 字面量）
//   node json.js assert-includes <file> <path> <json值>    # 数组包含
//   node json.js assert-match <file> <path> <正则>         # 序列化后匹配
const fs = require("fs");

const [mode, file, path, expected] = process.argv.slice(2);
if (!mode || !file) {
  console.error("用法: node json.js get|assert|assert-includes|assert-match <file> <path> [值]");
  process.exit(2);
}

const root = JSON.parse(fs.readFileSync(file, "utf8"));
let value = root;
if (path) {
  for (const key of path.split(".")) {
    if (value === null || value === undefined) break;
    value = value[key];
  }
}

const serialized = (v) => JSON.stringify(v);
const parseExpected = () => {
  try {
    return JSON.parse(expected);
  } catch {
    return expected;
  }
};

if (mode === "get") {
  if (value !== undefined && value !== null) {
    process.stdout.write(typeof value === "object" ? serialized(value) : String(value));
  }
} else if (mode === "assert") {
  if (serialized(value) !== serialized(parseExpected())) {
    console.error(`✗ 断言失败：${path} 期望 ${expected}，实际 ${serialized(value)}`);
    process.exit(1);
  }
} else if (mode === "assert-includes") {
  const needle = parseExpected();
  if (!Array.isArray(value) || !value.some((x) => serialized(x) === serialized(needle))) {
    console.error(`✗ 断言失败：${path} 不包含 ${expected}（实际 ${serialized(value)}）`);
    process.exit(1);
  }
} else if (mode === "assert-match") {
  if (!new RegExp(expected).test(serialized(value))) {
    console.error(`✗ 断言失败：${path} 不匹配 /${expected}/（实际 ${serialized(value)}）`);
    process.exit(1);
  }
} else {
  console.error(`未知模式 ${mode}`);
  process.exit(2);
}
