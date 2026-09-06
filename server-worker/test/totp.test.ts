import { describe, expect, it } from "vitest";
import { base32Encode, newTotpSecret, otpauthUrl, totpCodeAt, totpVerify } from "../src/crypto";

const enc = new TextEncoder();

describe("totp", () => {
  it("RFC 6238 附录 B 测试向量(密钥 12345678901234567890)", async () => {
    const secret = base32Encode(enc.encode("12345678901234567890"));
    expect(await totpCodeAt(secret, 59)).toBe("287082");
    expect(await totpCodeAt(secret, 1111111109)).toBe("081804");
    expect(await totpCodeAt(secret, 1234567890)).toBe("005924");
  });

  it("newTotpSecret 生成 32 字符的 base32 密钥", () => {
    const secret = newTotpSecret();
    expect(secret.length).toBe(32);
    expect(secret).toMatch(/^[A-Z2-7]{32}$/);
  });

  it("窗口 ±30 s 通过、+90 s 失败", async () => {
    const secret = newTotpSecret();
    const now = 1_700_000_000;
    const code = await totpCodeAt(secret, now);
    expect(await totpVerify(secret, code, now)).toBe(true);
    expect(await totpVerify(secret, code, now + 29)).toBe(true);
    expect(await totpVerify(secret, code, now - 30)).toBe(true);
    expect(await totpVerify(secret, code, now + 90)).toBe(false);
  });

  it("非 6 位数字直接判否", async () => {
    const secret = newTotpSecret();
    const now = 1_700_000_000;
    expect(await totpVerify(secret, "12345", now)).toBe(false);
    expect(await totpVerify(secret, "abcdef", now)).toBe(false);
  });

  it("非法密钥(非 base32 字符 / 空字符串)一律返回 false 而非抛错", async () => {
    const now = 1_700_000_000;
    expect(await totpVerify("!!!", "123456", now)).toBe(false);
    expect(await totpVerify("", "123456", now)).toBe(false);
  });

  it("otpauthUrl 格式正确", () => {
    const secret = newTotpSecret();
    expect(otpauthUrl(secret, "a@b.c")).toBe(
      `otpauth://totp/KiteTerm:a@b.c?secret=${secret}&issuer=KiteTerm&algorithm=SHA1&digits=6&period=30`,
    );
  });
});
