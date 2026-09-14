// 静态顺序检查：组件里「先用后声明」的 const 会导致运行时 TDZ 白屏
// （tsc 不会报，因为它只检查类型；空数据下运行时也可能不触发，测不到）。
// 这里用源码扫描把这类错误挡在提交前：
//   组件顶层（缩进 2 空格）声明的 const，其首次被引用的行号必须晚于声明行。

import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

const PAGES = ["../src/pages/Home.tsx", "../src/pages/Models.tsx"];

interface Decl {
  name: string;
  line: number;
}

/** 页面组件体范围：从 export default function 到下一个顶层 function / export function */
function pageComponentRange(lines: string[]): [number, number] {
  const start = lines.findIndex((l) => /^export default function /.test(l));
  if (start < 0) return [0, lines.length - 1];
  let end = lines.length - 1;
  for (let i = start + 1; i < lines.length; i += 1) {
    if (/^(export )?function /.test(lines[i]) || /^(export )?const [A-Z]/.test(lines[i])) {
      end = i - 1;
      break;
    }
  }
  return [start, end];
}

function topLevelConsts(lines: string[]): Decl[] {
  const decls: Decl[] = [];
  lines.forEach((line, index) => {
    // 只认组件顶层的 const（缩进恰好 2 空格），排除块内的局部变量
    const match = /^ {2}const ([A-Za-z_$][\w$]*)\s*=/.exec(line);
    if (match) decls.push({ name: match[1], line: index + 1 });
  });
  return decls;
}

function firstUse(lines: string[], name: string, declLine: number, from: number, to: number): number | null {
  const escaped = name.replace(/\$/g, "\\$");
  // 排除属性访问（.name / ?.name）与更长的标识符
  const pattern = new RegExp(`(?<![.?\\w$])${escaped}\\b`);
  for (let i = from; i <= to; i += 1) {
    const line = lines[i];
    if (i + 1 === declLine) continue; // 声明行自身不算使用
    const trimmed = line.trim();
    // 跳过 import / export 语句：路径里出现同名片段是误报
    if (trimmed.startsWith("import ") || trimmed.startsWith("export ")) continue;
    if (trimmed.includes('from "')) continue;
    // 跳过注释：注释里提到路径或术语是误报
    if (trimmed.startsWith("//") || trimmed.startsWith("*") || trimmed.startsWith("/*")) continue;
    // 跳过对象字面量键与类型属性定义（models: ... / price: ModelPrice | null）
    if (new RegExp(`(^\\s*|[{,]\\s*)${escaped}\\s*:`).test(line)) continue;
    if (pattern.test(line)) return i + 1;
  }
  return null;
}

test("页面组件里不存在「先用后声明」的 const（TDZ 白名单为空）", () => {
  const offenders: string[] = [];

  for (const relative of PAGES) {
    const file = fileURLToPath(new URL(relative, import.meta.url));
    const lines = readFileSync(file, "utf8").split("\n");
    const [from, to] = pageComponentRange(lines);
    for (const decl of topLevelConsts(lines.slice(from, to + 1))) {
      const absolute = { name: decl.name, line: decl.line + from };
      if (absolute.line > to + 1) continue;
      const use = firstUse(lines, absolute.name, absolute.line, from, to);
      if (use !== null && use < absolute.line) {
        offenders.push(
          `${relative}: '${absolute.name}' 在第 ${use} 行被使用，但第 ${absolute.line} 行才声明`
        );
      }
    }
  }

  assert.deepEqual(offenders, [], `发现先用后声明（会导致白屏）:\n${offenders.join("\n")}`);
});
