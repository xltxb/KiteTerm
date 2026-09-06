// 验证码邮件。配置了 RESEND_API_KEY 就走 Resend HTTP API,否则把验证码打进 Worker 日志 ——
// 小规模自建没有邮件服务时,管理员在控制台也能直接「标记已验证」。
import type { Env } from "./env";

/** 生成 6 位数字验证码(可能带前导零) */
export const newCode = () => String(Math.floor(Math.random() * 1_000_000)).padStart(6, "0");

/** 发送邮箱验证码;失败抛错,调用方按语义映射为 502 MAIL_ERROR 或忽略(注册路径走 waitUntil) */
export async function sendCode(env: Env, to: string, code: string): Promise<void> {
  const apiKey = env.RESEND_API_KEY;
  if (!apiKey) {
    console.log(`[mail] 未配置邮件服务 —— ${to} 的邮箱验证码: ${code}(10 分钟内有效)`);
    return;
  }
  const text = `你的 KiteTerm 邮箱验证码是 ${code},10 分钟内有效。若非本人操作请忽略。`;
  const res = await fetch("https://api.resend.com/emails", {
    method: "POST",
    headers: {
      authorization: `Bearer ${apiKey}`,
      "content-type": "application/json",
    },
    body: JSON.stringify({
      from: env.KITE_SMTP_FROM,
      to,
      subject: "KiteTerm 邮箱验证码",
      text,
    }),
  });
  if (!res.ok) {
    const body = await res.text().catch(() => "");
    throw new Error(`Resend 发送失败: ${res.status} ${body}`);
  }
}
