/**
 * KiteTerm 自定义线性图标集。
 * 与品牌标(KiteMark)同构:24 视口 / stroke-width 2 / round cap & join / currentColor。
 * 设计目标是替换应用内部分 lucide-react 导航/动作图标,保持整套视觉一致。
 *
 * 用法:<KtTerminal size={18} />
 */
import type { ReactNode, SVGProps } from "react";

interface IconProps extends SVGProps<SVGSVGElement> {
  size?: number;
}

function Icon({ size = 18, children, ...rest }: IconProps & { children: ReactNode }) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={2}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      {...rest}
    >
      {children}
    </svg>
  );
}

/* ---- 导航 ---- */

/** 终端 / 会话 */
export function KtTerminal(props: IconProps) {
  return (
    <Icon {...props}>
      <rect x="3" y="4.5" width="18" height="15" rx="3.5" />
      <path d="M7 9.5 l3 2.8 -3 2.8" />
      <path d="M13.5 15.2 h4" />
    </Icon>
  );
}

/** 主机 / 服务器 */
export function KtHosts(props: IconProps) {
  return (
    <Icon {...props}>
      <rect x="3.5" y="4.5" width="17" height="15" rx="3" />
      <path d="M3.5 9.5 h17" />
      <path d="M7 13 h.01" />
      <path d="M7 16.5 h.01" />
    </Icon>
  );
}

/** 传输 / 上载下载 */
export function KtTransfer(props: IconProps) {
  return (
    <Icon {...props}>
      <path d="M8 4 v14" />
      <path d="M5.5 6.5 L8 4 l2.5 2.5" />
      <path d="M16 20 v-14" />
      <path d="M13.5 17.5 L16 20 l2.5 -2.5" />
    </Icon>
  );
}

/** 端口转发 / 左右隧道 */
export function KtForward(props: IconProps) {
  return (
    <Icon {...props}>
      <path d="M4 8 h13" />
      <path d="M14 5 l3 3 -3 3" />
      <path d="M20 16 h-13" />
      <path d="M10 13 l-3 3 3 3" />
    </Icon>
  );
}

/** 监控 / 心跳 */
export function KtMonitor(props: IconProps) {
  return (
    <Icon {...props}>
      <path d="M3 12 h3.2 l2.6 6 3.4 -12 2.6 6 H21" />
    </Icon>
  );
}

/** 密钥 / 钥匙 */
export function KtKeys(props: IconProps) {
  return (
    <Icon {...props}>
      <circle cx="8" cy="8" r="4.2" />
      <path d="M11.2 11.2 L20 20" />
      <path d="M17 17 l3 -3" />
      <path d="M20 14 v3 h-3" />
    </Icon>
  );
}

/** 命令片段 / 花括号 */
export function KtSnippets(props: IconProps) {
  return (
    <Icon {...props}>
      <path d="M9 4 c-2.1 0 -3 1 -3 3.5 v2 c0 1.2 -.8 2 -2 2.5 1.2 .5 2 1.3 2 2.5 v2 c0 2.5 .9 3.5 3 3.5" />
      <path d="M15 4 c2.1 0 3 1 3 3.5 v2 c0 1.2 .8 2 2 2.5 -1.2 .5 -2 1.3 -2 2.5 v2 c0 2.5 -.9 3.5 -3 3.5" />
    </Icon>
  );
}

/** 云同步 / 循环刷新 */
export function KtSync(props: IconProps) {
  return (
    <Icon {...props}>
      <path d="M20 11.5 a7.5 7.5 0 1 0 -2.5 6.4" />
      <path d="M20 8 v4 h-4" />
    </Icon>
  );
}

/** 设置 / 滑杆 */
export function KtSettings(props: IconProps) {
  return (
    <Icon {...props}>
      <path d="M4 7 h12" />
      <circle cx="10" cy="7" r="2.2" />
      <path d="M4 12 h4" />
      <path d="M12 12 h8" />
      <circle cx="8" cy="12" r="2.2" />
      <path d="M4 17 h14" />
      <circle cx="15" cy="17" r="2.2" />
    </Icon>
  );
}

/* ---- 动作 ---- */

/** AI 协助 / 星光 */
export function KtAi(props: IconProps) {
  return (
    <Icon {...props}>
      <path d="M12 3 l1.9 5.1 L19 10 l-5.1 1.9 L12 17 l-1.9 -5.1 L5 10 l5.1 -1.9 z" />
      <path d="M19 15 l.7 1.9 1.9 .7 -1.9 .7 -.7 1.9 -.7 -1.9 -1.9 -.7 1.9 -.7 z" />
    </Icon>
  );
}

/** 搜索 / 放大镜 */
export function KtSearch(props: IconProps) {
  return (
    <Icon {...props}>
      <circle cx="11" cy="11" r="6.2" />
      <path d="M15.8 15.8 L21 21" />
    </Icon>
  );
}

/** 录制 / 实心圆点 */
export function KtRecord(props: IconProps) {
  return (
    <Icon {...props}>
      <circle cx="12" cy="12" r="8.5" />
      <circle cx="12" cy="12" r="3.4" fill="currentColor" stroke="none" />
    </Icon>
  );
}

/** 命令面板 / ⌘(可选,用于 ⌘K 交互) */
export function KtCommand(props: IconProps) {
  return (
    <Icon {...props}>
      <path d="M9 7 V5.5 A2.5 2.5 0 1 0 6.5 8 H9 Z" />
      <path d="M9 9 v6 H6.5 A2.5 2.5 0 1 0 9 17.5 V9 Z" />
      <path d="M15 7 V5.5 A2.5 2.5 0 1 1 17.5 8 H15 Z" />
      <path d="M15 9 v6 h2.5 A2.5 2.5 0 1 1 15 17.5 V9 Z" />
    </Icon>
  );
}

/** 品牌风筝标(与 KiteMark 同形) */
export function KtLogo(props: IconProps) {
  return (
    <Icon {...props}>
      <path d="M12 2.5 L18.4 9.2 L12 19.3 L5.6 9.2 Z" />
      <path d="M5.6 9.2 H18.4" />
      <path d="M12 19.3 c -.3 1.5 -1.7 1.9 -1.4 3.6" />
    </Icon>
  );
}
