/** 口令强度评估结果 */
export interface PasswordCheck {
  ok: boolean;
  score: number;
  hint: string | null;
}

/** 口令强度评估:与服务端 password_issue 的规则保持一致,先在本地给出即时反馈 */
export function checkPassword(pw: string): PasswordCheck {
  if (!pw) return { ok: false, score: 0, hint: null };
  const len = [...pw].length;
  const hasLetter = /\p{L}/u.test(pw);
  const hasDigit = /\p{N}/u.test(pw);
  const hasSymbol = /[^\p{L}\p{N}\s]/u.test(pw);
  const kinds = [hasLetter, hasDigit, hasSymbol].filter(Boolean).length;
  const weak = [
    "password", "password1", "password123", "12345678", "123456789", "1234567890",
    "qwerty123", "admin123", "abc12345", "iloveyou", "letmein1", "welcome1",
  ];
  if (len < 8) return { ok: false, score: Math.min(1, Math.floor(len / 4)), hint: "密码至少 8 位" };
  if (len > 128) return { ok: false, score: 1, hint: "密码最多 128 位" };
  if (kinds < 2) return { ok: false, score: 1, hint: "密码需包含字母、数字、符号中的至少两类" };
  if (new Set(pw).size < 4) return { ok: false, score: 1, hint: "密码字符过于单一" };
  if (weak.includes(pw.toLowerCase())) return { ok: false, score: 1, hint: "密码过于常见,请更换" };
  // 合格后按长度与字符种类给出 2~4 档强度
  let score = 2;
  if (len >= 12) score += 1;
  if (kinds === 3 && len >= 10) score += 1;
  return { ok: true, score: Math.min(4, score), hint: null };
}

const LABELS = ["很弱", "弱", "一般", "较强", "很强"];

/** 注册表单里的强度条:条宽与配色随分档变化,下方给出违规提示或平台规则 */
export function PasswordStrength({ check, rule }: { check: PasswordCheck; rule: string }) {
  return (
    <div className="mt-8">
      <div className="row gap-8">
        <div className={`progress thin spacer ${check.score >= 3 ? "t-success" : check.score >= 2 ? "t-warning" : "t-danger"}`}>
          <div style={{ width: `${check.score * 25}%` }} />
        </div>
        <span className="fs-xs t-faint text-right label-col sm">
          {LABELS[check.score]}
        </span>
      </div>
      <div className={`fs-xs mt-4 lh-normal ${check.ok ? "t-faint" : "t-warning"}`}>
        {check.hint ?? rule}
      </div>
    </div>
  );
}
