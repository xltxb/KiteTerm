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
