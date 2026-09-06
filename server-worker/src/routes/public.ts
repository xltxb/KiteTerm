import { Hono } from "hono";
import type { Env } from "../env";
import { featureGating, registrationDefault } from "../env";
import { settingGet } from "../settings";
import * as features from "../features";

export const publicRoutes = new Hono<{ Bindings: Env }>();

const PASSWORD_MIN_LEN = 8;
const PASSWORD_RULE = "至少 8 位,且包含字母、数字、符号中的至少两类";

publicRoutes.get("/api/health", (c) => c.json({ ok: true, service: "kite-server" }));

/**
 * 全局功能开关。免鉴权 —— 客户端未登录时也要能拿到,
 * 否则「登录开关」自身就没法下发。
 */
publicRoutes.get("/api/features", async (c) => {
  const gating = featureGating(c.env);
  const body = await features.payload(c.env.DB, gating);
  const registration = await settingGet(c.env.DB, "registration_mode", registrationDefault(c.env));
  return c.json({
    ...body,
    registration,
    enforced: gating,
    passwordMinLength: PASSWORD_MIN_LEN,
    passwordRule: PASSWORD_RULE,
  });
});

/** 公开的注册策略:客户端据此决定是否展示邀请码输入、以及口令规则提示 */
publicRoutes.get("/api/register/policy", async (c) => {
  const mode = await settingGet(c.env.DB, "registration_mode", registrationDefault(c.env));
  return c.json({ mode, passwordMinLength: PASSWORD_MIN_LEN, passwordRule: PASSWORD_RULE });
});
