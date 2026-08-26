import type { ButtonHTMLAttributes, CSSProperties, InputHTMLAttributes, ReactNode } from "react";
import { Loader2 } from "lucide-react";

type BtnVariant = "primary" | "secondary" | "outline" | "ghost" | "danger";

export function Button({
  variant = "primary",
  size = "sm",
  loading = false,
  children,
  ...rest
}: ButtonHTMLAttributes<HTMLButtonElement> & {
  variant?: BtnVariant;
  size?: "sm" | "md";
  loading?: boolean;
}) {
  return (
    <button className={`btn btn-${size} btn-${variant}`} disabled={loading || rest.disabled} {...rest}>
      {loading && <Loader2 size={14} style={{ animation: "vspin 0.9s linear infinite" }} />}
      {children}
    </button>
  );
}

export function TextInput({
  label,
  style,
  ...rest
}: InputHTMLAttributes<HTMLInputElement> & { label?: string }) {
  const input = <input className="input" style={label ? { width: "100%" } : style} {...rest} />;
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
  children,
  style,
}: {
  color?: "accent" | "success" | "warning" | "danger" | "neutral";
  dot?: boolean;
  children: ReactNode;
  style?: CSSProperties;
}) {
  return (
    <span className={`badge badge-${color}`} style={style}>
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

export function Progress({ value }: { value: number }) {
  return (
    <div className="progress">
      <div style={{ width: `${Math.max(0, Math.min(100, value))}%` }} />
    </div>
  );
}

export function ScreenHeader({
  eyebrow,
  title,
  count,
  children,
}: {
  eyebrow: string;
  title: string;
  count?: string;
  children?: ReactNode;
}) {
  return (
    <div style={{ display: "flex", alignItems: "flex-end", gap: 16, marginBottom: 22 }}>
      <div style={{ flex: 1 }}>
        <div className="eyebrow">{eyebrow}</div>
        <h1 className="screen-title">
          {title} {count && <span className="count">{count}</span>}
        </h1>
      </div>
      {children}
    </div>
  );
}
