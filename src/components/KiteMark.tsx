/**
 * KiteTerm 品牌标 —— 与应用图标同形的风筝。
 * 线性风格,笔画权重对齐 lucide-react(24 视口 / stroke-width 2 / round cap),
 * 以便和侧边栏其余导航图标视觉一致。
 */
export function KiteMark({ size = 18 }: { size?: number }) {
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
    >
      {/* 筝面:横骨在上 1/3,下半更长 —— 风筝而非菱形 */}
      <path d="M12 2 L18.4 8.4 L12 18.2 L5.6 8.4 Z" />
      {/* 十字骨架 */}
      <path d="M5.6 8.4 H18.4" />
      {/* 飘带 */}
      <path d="M12 18.2 c -0.4 1.5 -1.6 1.9 -1.4 3.6" />
    </svg>
  );
}
