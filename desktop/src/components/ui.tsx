import type { ButtonHTMLAttributes, CSSProperties, InputHTMLAttributes, ReactNode } from "react";
import { Loader2 } from "lucide-react";

type BtnVariant = "primary" | "secondary" | "outline" | "ghost" | "danger";

export function Button({
  variant = "primary",
  size = "sm",
  loading = false,
  className,
  children,
  ...rest
}: ButtonHTMLAttributes<HTMLButtonElement> & {
  variant?: BtnVariant;
  size?: "sm" | "md";
  loading?: boolean;
}) {
  // 调用方传的 className 是追加的修饰类(如 slim / w-full),不能覆盖掉基础 btn 类
  return (
    <button
      className={`btn btn-${size} btn-${variant}${className ? ` ${className}` : ""}`}
      disabled={loading || rest.disabled}
      {...rest}
    >
      {loading && <Loader2 size={14} className="spin" />}
      {children}
    </button>
  );
}

export function TextInput({
  label,
  className,
  style,
  ...rest
}: InputHTMLAttributes<HTMLInputElement> & { label?: string }) {
  const input = (
    <input
      className={`input${className ? ` ${className}` : ""}`}
      style={label ? { width: "100%" } : style}
      {...rest}
    />
  );
  if (!label) return input;
  return (
    <div style={style}>
      <label className="input-label">{label}</label>
      {input}
    </div>
  );
}

export function Badge({
  color = "neutral",
  dot = false,
  frame = false,
  className,
  children,
  style,
}: {
  color?: "accent" | "success" | "warning" | "danger" | "neutral";
  dot?: boolean;
  /** 方框描边式(蓝图风);默认是 pill */
  frame?: boolean;
  className?: string;
  children: ReactNode;
  style?: CSSProperties;
}) {
  return (
    <span
      className={`badge badge-${color}${frame ? " badge-frame" : ""}${className ? ` ${className}` : ""}`}
      style={style}
    >
      {dot && <span className="dot" />}
      {children}
    </span>
  );
}

export function Switch({ on, onChange }: { on: boolean; onChange: (on: boolean) => void }) {
  return (
    <button
      className="switch"
      role="switch"
      aria-checked={on}
      data-on={on}
      onClick={() => onChange(!on)}
    >
      <span className="thumb" />
    </button>
  );
}

export function Segmented<T extends string>({
  options,
  value,
  onChange,
  mono = false,
}: {
  options: { value: T; label: string }[];
  value: T;
  onChange: (v: T) => void;
  mono?: boolean;
}) {
  return (
    <div className="seg">
      {options.map((o) => (
        <div
          key={o.value}
          className={mono ? "seg-item mono" : "seg-item"}
          data-active={o.value === value}
          onClick={() => onChange(o.value)}
        >
          {o.label}
        </div>
      ))}
    </div>
  );
}

export function Progress({
  value,
  className,
  style,
}: {
  value: number;
  /** 追加修饰类,如 "thin"(细条,填充色取 currentColor,配合 style.color 换色) */
  className?: string;
  style?: CSSProperties;
}) {
  return (
    <div className={`progress${className ? ` ${className}` : ""}`} style={style}>
      <div style={{ width: `${Math.max(0, Math.min(100, value))}%` }} />
    </div>
  );
}

const SCREEN_ORDER = [
  "terminal", "hosts", "transfer", "forwards", "certs",
  "monitor", "keys", "snippets", "sync", "settings",
] as const;

export function ScreenHeader({
  eyebrow,
  title,
  count,
  screen,
  children,
}: {
  eyebrow: string;
  title: string;
  count?: string;
  /** 传了才显示序号;序号取自 types.ts 的 Screen 枚举顺序,表示真实位置 */
  screen?: (typeof SCREEN_ORDER)[number];
  children?: ReactNode;
}) {
  const idx = screen ? SCREEN_ORDER.indexOf(screen) + 1 : 0;
  return (
    <div className="screen-header">
      <div className="spacer">
        <div className="eyebrow">
          {eyebrow}
          {idx > 0 && (
            <span className="screen-ord">
              {String(idx).padStart(2, "0")}/{SCREEN_ORDER.length}
            </span>
          )}
        </div>
        <h1 className="screen-title">
          {title} {count && <span className="count">{count}</span>}
        </h1>
      </div>
      {children}
    </div>
  );
}

const READOUT_TONE = { ok: "success", warn: "warning", bad: "danger" } as const;

export function Readout({
  label,
  value,
  unit,
  tone,
}: {
  label: string;
  value: string | number;
  unit?: string;
  tone?: "ok" | "warn" | "bad";
}) {
  return (
    <div className="readout" data-tone={tone ? READOUT_TONE[tone] : undefined}>
      <span className="readout-label">{label}</span>
      <span className="readout-value">
        {value}
        {unit && <span className="readout-unit">{unit}</span>}
      </span>
    </div>
  );
}
