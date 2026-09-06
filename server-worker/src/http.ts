import type { Context } from "hono";
import type { Env } from "./env";
export type C = Context<{ Bindings: Env }>;
export const nowTs = () => Math.floor(Date.now() / 1000);
export function apiError(c: C, status: number, code: string, message: string) {
  return c.json({ error: { code, message } }, status as 400);
}
export function requireAdmin(c: C): Response | null {
  const key = c.req.header("x-admin-key") ?? "";
  if (!key || key !== c.env.KITE_ADMIN_KEY) return apiError(c, 401, "unauthorized", "管理密钥错误");
  return null;
}
export const bearerToken = (c: C) => c.req.header("authorization")?.replace(/^Bearer\s+/i, "") ?? null;
export const clientIp = (c: C) => c.req.header("cf-connecting-ip") ?? c.req.header("x-forwarded-for")?.split(",")[0]?.trim() ?? "unknown";
