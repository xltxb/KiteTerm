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
    return { error: "AI 给出的命令过长" };
  }
  // 一步就是一条命令。内嵌换行意味着多条 —— 用户批准时看到的很可能只是第一行,
  // 而批准路径不过 cmdguard 的 shell 结构检查。
  if (/[\n\r]/.test(command)) {
    return { error: "AI 给出的命令含换行,一步只能是一条命令" };
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
