/** AI 分步执行的响应解析自检。
 *
 *  模型返回的 JSON 常常不干净:包在 ```json 围栏里、前后带解释性文字、字段缺失。
 *  解析必须稳:拿不准就报错交回给用户,绝不猜一条命令出来执行。 */
import { parseAgentReply } from "./agentReply.ts";

let failed = 0;
function check(cond: boolean, msg: string) {
  if (!cond) { console.error("FAIL: " + msg); failed++; }
}

// 干净的 JSON
{
  const r = parseAgentReply('{"analysis":"a","next":{"intent":"i","command":"ls","readonly":true},"done":false}');
  check(!("error" in r), "干净 JSON 应解析成功");
  if (!("error" in r)) {
    check(r.analysis === "a", "analysis 正确");
    check(r.next?.command === "ls", "command 正确");
    check(r.next?.readonly === true, "readonly 正确");
    check(r.done === false, "done 正确");
  }
}

// 包在围栏里 —— 模型最常见的输出形态
{
  const r = parseAgentReply('```json\n{"analysis":"a","done":true,"summary":"s"}\n```');
  check(!("error" in r), "围栏包裹应能解析");
  if (!("error" in r)) check(r.summary === "s", "summary 正确");
}

// 前后带解释文字
{
  const r = parseAgentReply('好的,我来分析:\n{"analysis":"a","done":true,"summary":"s"}\n以上。');
  check(!("error" in r), "前后有文字时应能提取出 JSON");
}

// done=false 却没给 next —— 循环会卡住,必须报错
{
  const r = parseAgentReply('{"analysis":"a","done":false}');
  check("error" in r, "done=false 而无 next 应判为错误");
}

// readonly 缺失 —— 不能默认成 true,那是在放宽安全
{
  const r = parseAgentReply('{"analysis":"a","next":{"intent":"i","command":"ls"},"done":false}');
  check("error" in r || (!("error" in r) && r.next?.readonly === false),
    "readonly 缺失时必须按 false(需批准)处理,不可默认放行");
}

// 完全不是 JSON
{
  const r = parseAgentReply("我没法完成这个任务");
  check("error" in r, "非 JSON 应报错");
}

// 空
check("error" in parseAgentReply(""), "空输入应报错");

// readonly 必须严格等于 true —— 字符串 "true" 是真值,但不是布尔 true
{
  const r = parseAgentReply(
    '{"analysis":"a","next":{"intent":"i","command":"ls","readonly":"true"},"done":false}',
  );
  check(!("error" in r) && r.next?.readonly === false,
    'readonly 为字符串 "true" 时必须按 false(需批准)处理');
}
{
  const r = parseAgentReply(
    '{"analysis":"a","next":{"intent":"i","command":"ls","readonly":1},"done":false}',
  );
  check(!("error" in r) && r.next?.readonly === false, "readonly 为数字 1 时同样按 false 处理");
}

// done 含糊就报错,不能静默当成 false 继续往下跑
check("error" in parseAgentReply('{"analysis":"a","done":"true"}'),
  'done 为字符串 "true" 应报错');
check("error" in parseAgentReply(
  '{"analysis":"a","done":1,"next":{"intent":"i","command":"ls","readonly":true}}'),
  "done 为数字 1 应报错");

// 一步只能是一条命令
check("error" in parseAgentReply(
  '{"analysis":"a","done":false,"next":{"intent":"i","command":"ls\\n rm -rf /tmp/x","readonly":true}}'),
  "命令含换行应报错 —— 用户批准时可能只看到第一行");

// 超长命令
{
  const long = "a".repeat(5000);
  check("error" in parseAgentReply(
    `{"analysis":"a","done":false,"next":{"intent":"i","command":"${long}","readonly":true}}`),
    "超长命令应报错");
}

// U+2028/U+2029:JS 的 \n\r 正则不匹配,但浏览器渲染成换行 —— 同样能骗过眼睛
check("error" in parseAgentReply(
  '{"analysis":"a","done":false,"next":{"intent":"i","command":"ls\\u2028rm -rf /tmp/x","readonly":true}}'),
  "命令含 U+2028 行分隔符应报错");
check("error" in parseAgentReply(
  '{"analysis":"a","done":false,"next":{"intent":"i","command":"ls\\u2029rm -rf /tmp/x","readonly":true}}'),
  "命令含 U+2029 段分隔符应报错");

// NUL:渲染或跨 IPC 边界可能在此截断,后面的内容用户看不到却会执行
check("error" in parseAgentReply(
  '{"analysis":"a","done":false,"next":{"intent":"i","command":"ls\\u0000rm -rf /tmp/x","readonly":true}}'),
  "命令含 NUL 应报错");

// 零宽与双向控制字符能直接伪装命令原文(与 Rust 侧 cmdguard 的 is_deceptive 对齐)
check("error" in parseAgentReply(
  '{"analysis":"a","done":false,"next":{"intent":"i","command":"ls\\u202Etxt.elif","readonly":true}}'),
  "命令含 RTL override 应报错");

// 制表符是合法空白,不能误伤
{
  const r = parseAgentReply(
    '{"analysis":"a","done":false,"next":{"intent":"i","command":"grep\\tfoo\\tfile","readonly":true}}');
  check(!("error" in r), "制表符不该被拦");
}

if (failed > 0) { console.error(`agentReply 自检未通过:${failed} 项`); process.exit(1); }
console.log("AGENT_REPLY_PASS");
