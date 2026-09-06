import { settingGet, settingSet } from "./settings";
/** [名字, 默认值, 是否受门禁总闸] */
export const NAMES: ReadonlyArray<readonly [string, boolean, boolean]> = [
  ["login", true, true], ["sync", true, true], ["ai", true, true],
  ["key_sync", true, true], ["recording_sync", true, true],
  ["ai_command", true, true], ["ai_diagnose", true, true], ["ai_summary", true, true],
  ["email_verify", false, false], ["totp_required", false, false], ["multi_device", true, false],
];
const meta = (name: string) => NAMES.find(([n]) => n === name) ?? [name, true, true] as const;
export const camel = (s: string) => s.replace(/_([a-z])/g, (_, ch: string) => ch.toUpperCase());
const pascal = (s: string) => { const c = camel(s); return c[0].toUpperCase() + c.slice(1); };
export const snake = (pascalName: string) => pascalName.replace(/([A-Z])/g, (m, _c, i) => (i ? "_" : "") + m.toLowerCase());
export async function raw(db: D1Database, name: string): Promise<boolean> {
  const [, def] = meta(name);
  return (await settingGet(db, `feature_${name}`, def ? "1" : "0")) === "1";
}
export async function effective(db: D1Database, gating: boolean, name: string): Promise<boolean> {
  const [, , gated] = meta(name);
  if (gated && !gating) return true;
  return raw(db, name);
}
export const set = (db: D1Database, name: string, on: boolean) => settingSet(db, `feature_${name}`, on ? "1" : "0");
export async function payload(db: D1Database, gating: boolean): Promise<Record<string, boolean>> {
  const out: Record<string, boolean> = {};
  for (const [n] of NAMES) out[camel(n)] = await effective(db, gating, n);
  return out;
}
export async function adminPayload(db: D1Database): Promise<Record<string, boolean>> {
  const out: Record<string, boolean> = {};
  for (const [n] of NAMES) out[`feature${pascal(n)}`] = await raw(db, n);
  return out;
}
