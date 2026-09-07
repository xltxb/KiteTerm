import path from "node:path";
import { cloudflareTest, readD1Migrations } from "@cloudflare/vitest-plugin";
import { defineConfig } from "vitest/config";

export default defineConfig(async () => {
  const migrations = await readD1Migrations(path.join(__dirname, "migrations"));
  return {
    plugins: [
      cloudflareTest({
        wrangler: { configPath: "./wrangler.toml" },
        miniflare: {
          bindings: {
            TEST_MIGRATIONS: migrations,
            KITE_ADMIN_KEY: "test-admin-key",
            KITE_REGISTER_LIMIT: "3",
            // 测试按「门禁总闸默认关闭」编写;线上 wrangler.toml 里是 on,这里固定住不随之漂移
            KITE_FEATURE_GATING: "off",
          },
        },
      }),
    ],
    test: { setupFiles: ["./test/apply-migrations.ts"] },
  };
});
