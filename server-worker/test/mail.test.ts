import { describe, expect, it } from "vitest";
import { newCode } from "../src/mail";

describe("mail", () => {
  it("newCode 连续生成 200 个码,全部是 6 位数字", () => {
    for (let i = 0; i < 200; i++) {
      const code = newCode();
      expect(code).toMatch(/^\d{6}$/);
    }
  });
});
