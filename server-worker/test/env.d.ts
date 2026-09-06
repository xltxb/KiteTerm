/// <reference types="@cloudflare/vitest-plugin/types" />
import type { Env as WorkerEnv } from "../src/env";
import type { D1Migration } from "cloudflare:test";

// @cloudflare/workers-types 5.x 起,`cloudflare:workers` 的 `env`/`exports` 类型
// 来自全局命名空间 `Cloudflare.Env` / `Cloudflare.Exports`(而不是简报里旧版类型
// 直接对 "cloudflare:workers" 模块做接口增强的写法),`GlobalProps.mainModule`
// 用来推导 `exports.default` 的类型 —— 对应 `wrangler types` 生成的写法。
// 这里必须用 `declare global { namespace Cloudflare { ... } }` 包裹,并通过顶层
// `import type` 引入 Env(而不是在命名空间里内联 `import("../src/env").Env`):
// 实测内联 `import(...)` 类型会让 tsc 不把这份 .d.ts 计入全局作用域,导致声明合并失效。
declare global {
  namespace Cloudflare {
    interface Env extends WorkerEnv {
      TEST_MIGRATIONS: D1Migration[];
    }
    interface GlobalProps {
      mainModule: typeof import("../src/index");
    }
  }
}
export {};
