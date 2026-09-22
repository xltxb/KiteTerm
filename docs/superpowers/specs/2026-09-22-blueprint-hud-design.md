# 客户端视觉重做:蓝图 HUD 设计

日期:2026-09-22
状态:已确认
来源:用户要求「对客户端页面进行全面美化,搞的偏科技风」。经选型确认为**重做一套视觉语言**
      (而非把现有 Vela 用足),方向选定「蓝图 HUD」,浅色档配「晒图版」,改动深入到组件结构层。

## 目标

把 `desktop/` 客户端的视觉语言从「Vela 蓝 + 霓虹点缀」整体换成**工程蓝图 / 座舱 HUD**:
深青黑底 + 细网格 + 青色线框 + 近乎零圆角 + 角标刻线,数据以等宽读数呈现。
浅色档做成同一语言的反相——白底青墨线的工程晒图,而不是另一套设计。

## 非目标

- 不改任何业务逻辑、IPC 契约、store 结构。这是纯表现层改造。
- 不改间距与尺寸 token(`spacing.css`)。只换「表面」,不动布局节奏。
- 不改终端的渲染机制。`terminal.ts` 读 CSS 变量组装 xterm theme 的做法不动,只换取值。
- 不做主题切换器或多皮肤并存。蓝图取代 Vela,不与之共存。
- 不重画设计稿。本次之后实现即事实来源(见「遗留」)。

## 已定决策

| 决策 | 取值 | 理由 |
|---|---|---|
| 视觉方向 | 蓝图 HUD | 四个方向并排对照后用户选定;终端工具与工程图纸语汇天然契合 |
| 浅色档 | 晒图版(白底青墨线) | 蓝图本就有蓝底白线与白底蓝线两种真实形态,同一语言的两面 |
| 改动深度 | token + 组件样式 + 原语 | 用户选定最深一档 |
| JSX 改动范围 | 仅 `ui.tsx` 三处 | 角标/网格/线框纯 CSS 即可;为「改得彻底」而动 JSX 只增加回归面 |
| 正文字体 | 保持 Manrope + Noto Sans SC | 中文等宽长文本可读性差;HUD 感靠网格、线框、角标撑 |
| 标题字体 | 改用 JetBrains Mono 大写 | 顺带砍掉 Space Grotesk,少一个 webfont |
| 主按钮 | 全局唯一的实心元素 | 其余皆描边式,主次一眼分明 |

## 现状盘点

改造前实测,这些事实决定了方案形态:

| 事实 | 影响 |
|---|---|
| `.tsx` 中硬编码色值 **0 处** | 换色只需改 token 层 |
| `components.css` 硬编码 `border-radius` **26 处**,走 token 仅 21 处 | 换圆角前必须先收敛,否则 token 层失效 |
| `components.css` 硬编码色值 **1 处** | 色彩控制权已在 token 层 |
| `ui.tsx` 仅 154 行 / 7 个原语,全是 className 薄封装 | 改原语成本低,且 10 屏全部受益 |
| `--glow-md`(青霓虹)从未被引用 | 现有设计系统的科技感本就没用足 |
| `theme/terminal.test.ts` 是对比度契约测试,非值快照 | 换配色无需改测试,但必须守住对比度下限 |

## 第一层:token

### 色彩 — 深色档(蓝图主态)

| token | 值 |
|---|---|
| `--surface-page` | `#02141a` |
| `--surface-card` | `#04202a` |
| `--surface-sunken` | `#010e13` |
| `--surface-raised` | `#062a37` |
| `--border-subtle` | `rgba(34, 211, 238, 0.14)` |
| `--border-default` | `rgba(34, 211, 238, 0.26)` |
| `--border-strong` | `rgba(34, 211, 238, 0.50)` |
| `--text-strong` | `#ccf5ff` |
| `--text-body` | `#9fd8e6` |
| `--text-muted` | `#7fb9c8` |
| `--text-faint` | `#548a99` |
| `--accent` | `#22d3ee` |
| `--accent-hover` | `#67e8f9` |
| `--glow-accent` | `#22d3ee` |
| `--success` | `#a3e635` |
| `--warning` | `#fcd34d` |
| `--danger` | `#fb7185` |

边框带青色而非中性灰,是蓝图感的主要来源。成功态用黄绿而非绿,更贴仪表盘。

### 色彩 — 浅色档(晒图版)

| token | 值 |
|---|---|
| `--surface-page` | `#eef4f7` |
| `--surface-card` | `#ffffff` |
| `--border-subtle / default / strong` | `rgba(6, 90, 110, 0.16 / 0.28 / 0.48)` |
| `--text-strong / body / muted` | `#06323d` / `#0d4a5a` / `#4a7d8b` |
| `--accent` | `#0891b2` |
| `--success / warning / danger` | `#4d7c0f` / `#b45309` / `#be123c` |

强调色在白底上要深一档才够对比度,不能直接复用深色档的 `#22d3ee`。

### 效果

- 圆角:`--radius-md` 10px → **2px**,`--radius-lg` 14px → 3px,`--radius-xl` 20px → 4px,
  `--radius-sm` 6px → 2px,`--radius-xs` 4px → 1px。`--radius-full` 保留,供圆点与 pill 用。
- 新增 `--grid-size: 22px` 与 `--grid-line`(深 `rgba(34,211,238,.055)` / 浅 `rgba(6,90,110,.05)`)。
- `--glow-sm/md`:改为「1px 青描边 + 低扩散外发光」;浅色档去掉发光只留描边。
- `--shadow-*`:全部降档,深色档退化为描边。蓝图风不用投影。

### 字体角色

| 角色 | 字体 |
|---|---|
| 标题 / 标签 / eyebrow | JetBrains Mono,大写,`letter-spacing: .08em` |
| 正文 | Manrope + Noto Sans SC(不变) |
| 数据(端口/延迟/字节/时间) | JetBrains Mono + `tabular-nums` |

`fonts.css` 的 `@import` 去掉 Space Grotesk,`--font-heading` 指向 mono 栈。

## 第二层:组件样式

### 0. 前置清理(零视觉变化)

把 `components.css` 的 26 处硬编码 `border-radius` 收敛到 token:`99px` → `--radius-full`
(pill 与圆点保持全圆),`14px/10px/8px` → `--radius-lg/md/sm`。此步在 token 值仍为旧值时进行,
不产生任何视觉变化,纯粹把控制权交还 token 层。

### 1. 网格底

挂在 `.app-main`,不挂全局——侧栏与状态栏保持实色面板,网格只在内容区透出,层次才分得开。

```css
.app-main {
  background-color: var(--surface-page);
  background-image:
    linear-gradient(var(--grid-line) 1px, transparent 1px),
    linear-gradient(90deg, var(--grid-line) 1px, transparent 1px);
  background-size: var(--grid-size) var(--grid-size);
}
.screen { background: transparent; }
```

用 `background-image` 而非伪元素,避免与现有 flex 布局的层叠冲突。

### 2. 角标

`::before` 画左上、`::after` 画右下,对角呼应。**只给 `.card` 与 `.panel` 用**——
每个盒子都上会满屏刻线,反而廉价。

```css
.card { position: relative; }
.card::before, .card::after {
  content: ""; position: absolute; width: 6px; height: 6px;
  border-color: var(--accent); opacity: .55; pointer-events: none;
}
.card::before { top: -1px; left: -1px; border-top: 1px solid; border-left: 1px solid; }
.card::after  { bottom: -1px; right: -1px; border-bottom: 1px solid; border-right: 1px solid; }
```

### 3. 逐类改法

| 类 | 改动 |
|---|---|
| `.card` | 去 `box-shadow`,边框转青,圆角走 token,加角标 |
| `.panel-head` | 底部加 1px 青分隔线,`.panel-title` 转 mono 大写 |
| `.btn-primary` | 青实底 + 深色字,全局唯一实心元素 |
| `.btn-secondary` / `.btn-outline` | 青描边 + 透明底 |
| `.nav-item[data-active="true"]` | 左侧 2px 青竖条 + 极淡青底 + 青字,取代现有圆角块。侧栏有折叠态(`.sidebar[data-collapsed="true"]`,此时只剩图标居中),竖条需单独处理 |
| `.statusbar` | 已是 mono;加青色竖分隔符,数值加 `tabular-nums` |
| `.badge` | 去圆角变方,改描边式 |
| `.input` | 去圆角,focus 时边框转青 + 细发光 |
| `.eyebrow` | 不改。它已是 mono + 大写 + 字距 + accent 色 |

### 4. 明确不动

`.term-*`(归 `terminal.css` 管)、`.modal-*` / `.sheet-*` 的布局尺寸、`.progress` 的动画逻辑、
全部间距 token。

## 第三层:原语与屏幕

### `ui.tsx` 的三处改动

**`ScreenHeader`** — 加一条右侧延伸的刻度基线与屏幕编号(如 `01/10`)。编号取自侧栏导航的实际
位置,**必须表示真实序号**,不可为装饰。一处改动,10 屏受益(`Screen` 类型枚举共 10 项)。

**新增 `Readout`** — 数据读数原语:

```tsx
<Readout label="延迟" value="12" unit="ms" tone="ok" />
```

统一「标签 + 等宽数值 + 单位」的排版与 `tabular-nums`。现状是各屏手写 `<span>` 拼,对不齐。
复用已存在的 `.num-readout` / `.stat-value` 类。

**`Badge`** — 新增 `frame` 变体(方框描边式);原 pill 形态保留给状态点。

其余四个原语(`Button` `TextInput` `Switch` `Progress` `Segmented`)**一行不改**,纯靠 CSS 换装。

### 屏幕铺开顺序

| 批次 | 屏幕 | 理由 |
|---|---|---|
| 1 | 终端屏 + `RightDock` + `statusbar` | HUD 感主场,使用频率最高 |
| 2 | 主机、转发、监控 | 数据密集,最吃读数原语 |
| 3 | 传输、证书、密钥、片段 | 列表为主,随组件层自动变 |
| 4 | 设置、同步 | 表单为主,token 层即足够 |

批次 3、4 很可能无需专门改代码;前两层做完后按实际效果再定。

## 测试

| 测试 | 影响 |
|---|---|
| `theme/terminal.test.ts` | **不改**。它是契约测试而非值快照,见下 |
| `nlDetect` / `dialog` / `errorDetect` / `broadcast` | 不碰样式,不受影响 |
| Rust 全部(91 + 2 ignored) | 完全不受影响 |

`terminal.test.ts` 解析 `terminal.css` 并**离线计算对比度**,断言的是契约不是取值:

1. 深浅两档必须覆盖同一组 `--term-*` 键(缺一个 xterm 就退回自己的默认色);
2. 浅色档底色 luminance ≥ 0.5,深色档 ≤ 0.1;
3. 每档所有前景色对本档底色:ANSI 色 ≥ 3:1,`--term-fg` ≥ 4.5:1;
4. 豁免与画布同侧的那一端(浅色档的 `white`/`bright-white`、深色档的 `black`)——
   这是 ANSI 固有语义,程序拿它当背景色用。

所以新配色**不需要改任何断言**:守住契约就自动通过,守不住就该改配色而不是改测试。
这道关反而会替我们挡住「亮色放到白底上看不见」这类问题。

ANSI 十六色仍需重挑并已逐项验算(见实施计划),其中浅色档 `bright-green` 与
`bright-yellow` 的首版取值分别只有 2.91:1 和 3.00:1,已调深至达标。

## 提交切分

1. 圆角收敛到 token(零视觉变化,纯重构)
2. token 重写(`terminal.test.ts` 不改,由它验证新配色)
3. `components.css`:网格、线框、角标
4. `ui.tsx` 三处原语 + 批次 1、2 屏幕

每个提交后 `pnpm test` 与 `cargo test` 全绿再进下一个。

## 验证方式

`pnpm tauri dev` 常驻,CSS 改动热重载,边改边看。自动化只能保证不回归,视觉需人工确认。

## 风险与遗留

- **与设计稿脱钩。** Claude Design 项目 `d49d6720` 的 Vela Design System 与两份设计稿在本次之后
  不再是实现依据。要么后续补一版蓝图版设计稿,要么接受实现成为唯一事实来源。当前选择后者。
- **浅色档的实际观感未经验证。** 晒图版是推演出来的,真跑起来可能偏冷或对比过强,需在第二个提交
  后专门看一遍浅色档。
- **`fonts.css` 用 `@import` 从 Google Fonts 拉字体,离线时静默回退系统字体。** 这是既有问题,
  不在本次范围。要根治需把字体内嵌进 Tauri 包,另开一件事。
