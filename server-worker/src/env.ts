export interface Env {
  DB: D1Database;
  RATE: KVNamespace;
  KITE_ADMIN_KEY: string;
  KITE_REGISTRATION?: string;
  KITE_FEATURE_GATING?: string;
  KITE_REGISTER_LIMIT?: string;
  S3_ENDPOINT?: string; S3_BUCKET?: string; S3_REGION?: string; S3_PREFIX?: string; S3_PATH_STYLE?: string;
  S3_ACCESS_KEY?: string; S3_SECRET_KEY?: string;
  KITE_STS?: string;
  R2_ACCOUNT_ID?: string; R2_API_TOKEN?: string;
  RESEND_API_KEY?: string; KITE_SMTP_FROM?: string;
}
export const featureGating = (env: Env) => ["on", "1", "true"].includes((env.KITE_FEATURE_GATING ?? "off").toLowerCase());
export const registrationDefault = (env: Env) => (["open", "invite", "closed"].includes(env.KITE_REGISTRATION ?? "") ? env.KITE_REGISTRATION! : "open");

/** 注册限流默认上限(与 Rust 版 REGISTER_MAX_PER_WINDOW 一致) */
const REGISTER_LIMIT_DEFAULT = 5;

/**
 * 同一 IP 每窗口的注册尝试上限。与 Rust 版 `register_limit()` 同语义:只认非负整数,
 * 空串 / 非数字 / 负数 / 小数一律回退默认值。
 *
 * 不能直接 `Number(...)` —— 非数字会变成 `NaN`,而 `hits.length >= NaN` 恒为 false,
 * 限流会被静默关掉;空串会变成 0,反过来把注册全部挡死。
 */
export const registerLimit = (env: Env) =>
  /^\d+$/.test(env.KITE_REGISTER_LIMIT ?? "") ? Number(env.KITE_REGISTER_LIMIT) : REGISTER_LIMIT_DEFAULT;
