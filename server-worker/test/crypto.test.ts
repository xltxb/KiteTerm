import { describe, expect, it } from "vitest";
import { hashPassword, timingSafeEqual, verifyPassword } from "../src/crypto";

describe("crypto", () => {
  it("hashPassword/verifyPassword 往返成功,格式前缀正确", async () => {
    const stored = await hashPassword("Abc12345");
    expect(stored.startsWith("pbkdf2$100000$")).toBe(true);
    expect(await verifyPassword("Abc12345", stored)).toBe(true);
  });

  it("错误口令验证失败", async () => {
    const stored = await hashPassword("Abc12345");
    expect(await verifyPassword("wrong-password", stored)).toBe(false);
  });

  it("timingSafeEqual 长度不等直接返回 false", () => {
    expect(timingSafeEqual(new Uint8Array([1, 2, 3]), new Uint8Array([1, 2]))).toBe(false);
    expect(timingSafeEqual(new Uint8Array([1, 2, 3]), new Uint8Array([1, 2, 3]))).toBe(true);
    expect(timingSafeEqual(new Uint8Array([1, 2, 3]), new Uint8Array([1, 2, 4]))).toBe(false);
  });
});
