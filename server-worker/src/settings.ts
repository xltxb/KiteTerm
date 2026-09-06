export async function settingGet(db: D1Database, key: string, def: string): Promise<string> {
  const row = await db.prepare("SELECT value FROM platform_settings WHERE key = ?1").bind(key).first<{ value: string }>();
  return row?.value ?? def;
}
export async function settingSet(db: D1Database, key: string, value: string): Promise<void> {
  await db.prepare("INSERT INTO platform_settings (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = ?2").bind(key, value).run();
}
