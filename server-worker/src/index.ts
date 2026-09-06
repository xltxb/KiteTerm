import { Hono } from "hono";
import type { Env } from "./env";
import { publicRoutes } from "./routes/public";
import { adminRoutes } from "./routes/admin";
const app = new Hono<{ Bindings: Env }>();
app.route("/", publicRoutes);
app.route("/", adminRoutes);
app.notFound((c) => c.json({ error: { code: "not_found", message: "接口不存在" } }, 404));
export default {
  fetch: app.fetch,
  // 简报里写的是旧版 `ScheduledEvent`;当前安装的 @cloudflare/workers-types@5 里
  // `ExportedHandler.scheduled` 的第一个参数类型是 `ScheduledController`,按实际类型调整。
  async scheduled(_c: ScheduledController, env: Env) {
    await env.DB.prepare("DELETE FROM tokens WHERE expires_at < ?1").bind(Math.floor(Date.now() / 1000)).run();
  },
};
