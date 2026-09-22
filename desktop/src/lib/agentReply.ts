/** AI 分步执行的响应契约与解析。
 *
 *  模型返回的 JSON 常常不干净:包在 ```json 围栏里、前后带解释性文字。解析取
 *  最外层的一对花括号来容错,但字段校验必须严 —— 拿不准就报错交回用户,绝不
 *  猜一条命令出来执行。
 *
 *  安全相关的一条:readonly 缺失时按 false(需批准)处理。默认成 true 等于
 *  在放宽安全边界,而这个字段本就只能让自动执行更少、不能更多。 */

/** 一条命令的长度上限 */
const MAX_COMMAND_LEN = 4096;

/** 会让命令显示得与实际不符的字符。
 *
 *  换行类(含 U+2028/U+2029 —— JS 的 \n\r 不匹配它们,但浏览器照样渲染成换行)
 *  能让用户只看到第一行就批准,实际执行的却是多条;零宽与双向控制字符能直接伪装
 *  命令原文;NUL 可能让渲染在中途截断。
 *
 *  保留 \t(U+0009)—— 制表符在命令里是合法的空白。
 *
 *  这与 Rust 侧 cmdguard.rs 的 is_deceptive 是同一套字符集:那边守自动执行路径,
 *  这边守批准路径(批准的命令不过 cmdguard)。 */
const DECEPTIVE =
  /[\u0000-\u0008\u000A-\u001F\u007F  ​-‏‪-‮⁦-⁩﻿]/;

export interface AgentNext {
  intent: string;
  command: string;
  readonly: boolean;
}

export interface AgentReply {
  analysis: string;
  next?: AgentNext;
  done: boolean;
  summary?: string;
}

function extractJson(raw: string): string | null {
  const start = raw.indexOf("{");
  const end = raw.lastIndexOf("}");
  if (start < 0 || end <= start) return null;
  return raw.slice(start, end + 1);
}

export function parseAgentReply(raw: string): AgentReply | { error: string } {
  const text = extractJson(raw ?? "");
  if (!text) return { error: "AI 没有返回可解析的 JSON" };

  let obj: unknown;
  try {
    obj = JSON.parse(text);
  } catch {
    return { error: "AI 返回的 JSON 无法解析" };
  }
  if (typeof obj !== "object" || obj === null) return { error: "AI 返回的不是对象" };

  const o = obj as Record<string, unknown>;
  const analysis = typeof o.analysis === "string" ? o.analysis : "";
  // done 必须是明确的布尔值。含糊就报错 —— readonly 含糊时落到 false 是收紧,
  // done 含糊时落到 false 却是「继续执行下一步」,方向相反,不能同样静默处理。
  if (o.done !== undefined && typeof o.done !== "boolean") {
    return { error: "AI 返回的 done 字段不是布尔值" };
  }
  const done = o.done === true;
  const summary = typeof o.summary === "string" ? o.summary : undefined;

  if (done) return { analysis, done: true, summary };

  const n = o.next;
  if (typeof n !== "object" || n === null) {
    return { error: "AI 说任务未完成,却没给出下一步" };
  }
  const nn = n as Record<string, unknown>;
  const command = typeof nn.command === "string" ? nn.command.trim() : "";
  if (!command) return { error: "AI 给出的下一步没有命令" };
  if (command.length > MAX_COMMAND_LEN) {
    return { error: `AI 给出的命令过长(${command.length} 字符,上限 ${MAX_COMMAND_LEN})` };
  }
  if (DECEPTIVE.test(command)) {
    return { error: "AI 给出的命令含换行或不可见字符,一步只能是一条命令" };
  }

  return {
    analysis,
    done: false,
    next: {
      intent: typeof nn.intent === "string" ? nn.intent : "",
      command,
      // 缺失或非 true 一律按需批准 —— 不可默认放行
      readonly: nn.readonly === true,
    },
  };
}
