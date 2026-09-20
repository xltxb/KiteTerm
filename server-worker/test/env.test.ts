import { describe, expect, it } from "vitest";
import type { Env } from "../src/env";
import { registerLimit } from "../src/env";

const withLimit = (v?: string) => ({ KITE_REGISTER_LIMIT: v }) as Env;

describe("registerLimit", () => {
  it("未设置 → 默认 5", () => {
    expect(registerLimit({} as Env)).toBe(5);
  });

  it("非负整数 → 原值", () => {
    expect(registerLimit(withLimit("10"))).toBe(10);
    // 与 Rust 版一致:显式配 0 就是「一次都不放行」,不是「不限流」
    expect(registerLimit(withLimit("0"))).toBe(0);
  });

  // 这几种此前都会出事:Number("abc") = NaN,而 hits.length >= NaN 恒为 false,
  // 限流被静默关掉;Number("") = 0,反过来把注册全部挡死。
  it.each(["abc", "", " 10", "5.5", "-1"])("非法值 %j → 回退 5", (v) => {
    expect(registerLimit(withLimit(v))).toBe(5);
  });
});
