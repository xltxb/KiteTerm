# 客户端蓝图 HUD 视觉重做 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 `desktop/` 客户端的视觉语言整体换成工程蓝图 / 座舱 HUD——深青黑底 + 细网格 + 青色线框 + 近乎零圆角 + 角标刻线,浅色档做成同一语言反相的白底青墨线晒图。

**Architecture:** 三层自底向上。第一层只换 `theme/` 的 token 取值(色彩、圆角、网格、字体角色),因为 `.tsx` 里 0 处硬编码色值,换 token 即全局生效;但换圆角前必须先把 `components.css` 里 26 处硬编码 `border-radius` 收敛到 token,否则 token 层管不住。第二层在 `components.css` 加网格底、线框卡片与角标,纯 CSS。第三层只动 `ui.tsx` 的三处原语(`ScreenHeader` 加刻度基线与真实序号、新增 `Readout`、`Badge` 加 `frame` 变体)——角标和网格纯 CSS 就能拿到,为「改得彻底」而动更多 JSX 只会增加回归面。

**Tech Stack:** Tauri 2 + React 18 + Zustand;样式为手写 CSS + 语义 token(`desktop/src/theme/`),无 CSS 框架;测试是 `node --experimental-strip-types` 直接跑的 `.test.ts` 脚本。

**Spec:** `docs/superpowers/specs/2026-09-22-blueprint-hud-design.md`

## Global Constraints

- 样式只用 `desktop/src/theme/` 的 token 与 `utilities.css` 工具类,**禁止新增硬编码色值**(现状 `.tsx` 0 处、`components.css` 1 处,不得增加)。
- 不改任何业务逻辑、IPC 契约、store 结构。这是纯表现层改造。
- 不改 `spacing.css` 的间距与尺寸 token,不动布局节奏。
- 不改 `terminal.ts` 读 CSS 变量组装 xterm theme 的机制,只换 `terminal.css` 的取值。
- **不改 `theme/terminal.test.ts`。** 它是契约测试(解析 CSS 离线算对比度),新配色必须守住:两档键一致、浅底 luminance ≥ 0.5 / 深底 ≤ 0.1、ANSI ≥ 3:1、`--term-fg` ≥ 4.5:1。守不住就改配色,不改测试。
- 每个任务结束前 `cd desktop && pnpm test` 必须全绿;涉及 Rust 的任务另跑 `cargo test --manifest-path desktop/src-tauri/Cargo.toml`(本计划预期不涉及)。
- 提交:每个任务末尾的 commit 步骤只在用户确认允许提交时执行;否则跳过该步,保持工作区改动。
- 验证:`pnpm tauri dev` 常驻,CSS 热重载。自动化只保证不回归,**视觉必须人工看过**。

---

## 文件结构

| 文件 | 责任 | 本次改动 |
|---|---|---|
| `desktop/src/theme/colors.css` | 色板与语义别名,深浅两档 | 重写语义段取值(任务 2) |
| `desktop/src/theme/effects.css` | 圆角、阴影、发光、模糊 | 圆角降档 + 新增网格 token + 发光改描边式(任务 2) |
| `desktop/src/theme/terminal.css` | 终端 ANSI 十六色,深浅两档 | 重配(任务 3) |
| `desktop/src/theme/fonts.css` | webfont 引入 | 去 Space Grotesk(任务 4) |
| `desktop/src/theme/typography.css` | 字体栈与字号 token | `--font-display` 改等宽打头 + 保留 CJK 回退(任务 4) |
| `desktop/src/theme/components.css` | 113 个组件类 | 圆角收敛(任务 1)、网格/线框/角标/逐类改法(任务 5、6) |
| `desktop/src/components/ui.tsx` | 7 个 UI 原语 | 三处改动(任务 7) |
| `desktop/src/app/Sidebar.tsx` | 侧栏导航 | 仅供任务 7 读取序号,**不改** |

---

## Task 1: 圆角收敛到 token(零视觉变化)

把 `components.css` 里 26 处硬编码 `border-radius` 换成 token 引用。此时 token 取值仍是旧的,所以**渲染结果必须逐像素不变**——这是纯重构,为任务 2 的圆角降档铺路。

**Files:**
- Modify: `desktop/src/theme/components.css`(26 处,行号见下)

**Interfaces:**
- Consumes: `effects.css` 已有的 `--radius-xs/sm/md/lg/xl/full`
- Produces: `components.css` 中不再有裸 `border-radius: <数字>px`(多值简写除外,见步骤 3)

映射规则(依据 `effects.css` 现值 `xs:4 sm:6 md:10 lg:14 xl:20 full:999`):

| 原值 | 换成 |
|---|---|
| `99px` | `var(--radius-full)` |
| `14px` | `var(--radius-lg)` |
| `12px` | `var(--radius-lg)` |
| `10px` | `var(--radius-md)` |
| `8px` | `var(--radius-sm)` |
| `6px` | `var(--radius-sm)` |

`12px` 归到 `lg`(14px)、`6px` 归到 `sm`(6px 精确命中)。`12px → 14px` 是本任务**唯一**的像素变化,发生在第 897 行与两处气泡圆角上;因为任务 2 会把 `lg` 降到 3px,这点差异不会留到最后。若希望严格零变化,可先给 `effects.css` 加 `--radius-lg-alt: 12px`——**不推荐**,徒增一个即将废弃的 token。

- [ ] **Step 1: 记录改动前的基线**

```bash
cd /Users/abiu/GolandProjects/project/KiteTerm/desktop
grep -cE 'border-radius:\s*[0-9]+px' src/theme/components.css   # 期望输出 26
grep -c 'border-radius:\s*var(--radius' src/theme/components.css # 期望输出 21
```

- [ ] **Step 2: 替换单值圆角**

除第 686、695 行(多值简写)外的 24 处,按上表逐个替换。可用脚本,但**替换后必须人工核对 diff**:

```bash
cd /Users/abiu/GolandProjects/project/KiteTerm/desktop/src/theme
python3 - <<'PY'
import pathlib, re
p = pathlib.Path('components.css')
s = p.read_text()
M = {'99px':'var(--radius-full)','14px':'var(--radius-lg)','10px':'var(--radius-md)',
     '8px':'var(--radius-sm)','6px':'var(--radius-sm)','12px':'var(--radius-lg)'}
def sub(m):
    v = m.group(1)
    return f'border-radius: {M[v]};' if v in M else m.group(0)
s = re.sub(r'border-radius:\s*([0-9]+px);', sub, s)
p.write_text(s)
PY
git diff --stat src/theme/components.css
```

- [ ] **Step 3: 手工处理两处多值简写**

第 686、695 行是气泡的不对称圆角,脚本不会碰(它们带多个值)。手工改:

```css
/* 原: border-radius: 12px 12px 4px 12px; */
border-radius: var(--radius-lg) var(--radius-lg) var(--radius-xs) var(--radius-lg);

/* 原: border-radius: 12px 12px 12px 4px; */
border-radius: var(--radius-lg) var(--radius-lg) var(--radius-lg) var(--radius-xs);
```

- [ ] **Step 4: 验证收敛干净**

```bash
cd /Users/abiu/GolandProjects/project/KiteTerm/desktop
grep -nE 'border-radius:[^;]*[0-9]+px' src/theme/components.css
```

Expected: 无输出。有输出说明还有漏网的,逐个处理。

- [ ] **Step 5: 跑测试**

```bash
cd /Users/abiu/GolandProjects/project/KiteTerm/desktop && pnpm test
```

Expected: 全部通过,含 `TERMINAL_THEME_PASS(浅色 21 项 / 深色 21 项)`。本任务不碰配色,这行输出应与改动前完全一致。

- [ ] **Step 6: 人工看一眼**

dev 窗口应**几乎无变化**(仅气泡与两处 12px 圆角微增 2px)。若看到明显变化,说明映射表用错了,回退重来。

- [ ] **Step 7: 提交**

```bash
cd /Users/abiu/GolandProjects/project/KiteTerm
git add desktop/src/theme/components.css
git commit -m "refactor(desktop): 组件圆角收敛到 token

26 处硬编码 border-radius 换成 --radius-* 引用,比走 token 的 21 处还多,
不收敛的话后面调圆角 token 根本管不住这些类。此步不改 token 取值,渲染
基本不变(仅 12px 归入 lg 档微增 2px,且 lg 马上要降到 3px)。"
```

---

## Task 2: 重写色彩与效果 token

一次换掉深浅两档的全部语义色、圆角档位,并新增网格 token。这一步视觉变化最大。

**Files:**
- Modify: `desktop/src/theme/colors.css`(语义段,`SEMANTIC — Light` 与 `[data-theme="dark"]`)
- Modify: `desktop/src/theme/effects.css`(圆角、发光、阴影,新增网格)

**Interfaces:**
- Consumes: 任务 1 收敛后的 `components.css`
- Produces: `--grid-size`、`--grid-line` 两个新 token,供任务 5 的网格底使用

- [ ] **Step 1: 改 `effects.css` 的圆角与网格**

```css
:root {
  /* ---- Corner radii(蓝图:近乎直角,pill 与圆点例外) ---- */
  --radius-xs:   1px;
  --radius-sm:   2px;
  --radius-md:   2px;
  --radius-lg:   3px;
  --radius-xl:   4px;
  --radius-2xl:  6px;
  --radius-full: 999px;

  /* ---- 图纸网格 ---- */
  --grid-size: 22px;
  --grid-line: rgba(6, 90, 110, 0.05);
}

[data-theme="dark"] {
  --grid-line: rgba(34, 211, 238, 0.055);
}
```

- [ ] **Step 2: 改 `effects.css` 的发光与阴影**

蓝图风不用投影。浅色档保留极轻阴影(晒图纸张感),深色档退化为描边:

```css
:root {
  --shadow-xs: 0 1px 1px rgba(6, 45, 56, 0.05);
  --shadow-sm: 0 1px 2px rgba(6, 45, 56, 0.07);
  --shadow-md: 0 2px 6px rgba(6, 45, 56, 0.09);
  --shadow-lg: 0 6px 16px rgba(6, 45, 56, 0.12);
  --shadow-xl: 0 12px 32px rgba(6, 45, 56, 0.16);

  --glow-sm: 0 0 0 1px rgba(8, 145, 178, 0.35);
  --glow-md: 0 0 0 1px rgba(8, 145, 178, 0.55);
}

[data-theme="dark"] {
  --shadow-xs: none;
  --shadow-sm: 0 0 0 1px rgba(34, 211, 238, 0.10);
  --shadow-md: 0 0 0 1px rgba(34, 211, 238, 0.14);
  --shadow-lg: 0 0 0 1px rgba(34, 211, 238, 0.18), 0 8px 24px rgba(0, 0, 0, 0.5);
  --shadow-xl: 0 0 0 1px rgba(34, 211, 238, 0.22), 0 16px 40px rgba(0, 0, 0, 0.6);

  --glow-sm: 0 0 0 1px rgba(34, 211, 238, 0.45), 0 0 12px rgba(34, 211, 238, 0.18);
  --glow-md: 0 0 0 1px rgba(34, 211, 238, 0.70), 0 0 20px rgba(34, 211, 238, 0.28);
}
```

- [ ] **Step 3: 改 `colors.css` 的浅色语义段(晒图版)**

替换 `SEMANTIC — Light (default)` 段中这些键的取值,**其余键不动**:

```css
  --surface-page:    #eef4f7;
  --surface-card:    #ffffff;
  --surface-sunken:  #e4eef2;
  --surface-raised:  #ffffff;
  --surface-inverse: #06323d;
  --surface-overlay: rgba(6, 50, 61, 0.45);
  --surface-hover:   rgba(6, 90, 110, 0.06);

  --border-subtle:   rgba(6, 90, 110, 0.16);
  --border-default:  rgba(6, 90, 110, 0.28);
  --border-strong:   rgba(6, 90, 110, 0.48);

  --text-strong:     #06323d;
  --text-body:       #0d4a5a;
  --text-muted:      #4a7d8b;
  --text-faint:      #7fa3ad;
  --text-on-accent:  #ffffff;
  --text-inverse:    #eef4f7;

  --accent:          #0891b2;
  --accent-hover:    #0e7490;
  --accent-pressed:  #155e75;
  --accent-subtle:   #e0f2f7;
  --accent-subtle-border: rgba(8, 145, 178, 0.30);
  --accent-text:     #0e7490;

  --glow-accent:     #0891b2;

  --success:         #4d7c0f;
  --success-text:    #3f6212;
  --success-subtle:  #eef7e0;
  --warning:         #b45309;
  --warning-text:    #92400e;
  --warning-subtle:  #fdf0e0;
  --danger:          #be123c;
  --danger-text:     #9f1239;
  --danger-subtle:   #fdeaee;

  --focus-ring:      rgba(8, 145, 178, 0.45);
  --selection-bg:    #cdeaf2;
```

- [ ] **Step 4: 改 `colors.css` 的深色语义段(蓝图主态)**

```css
  --surface-page:    #02141a;
  --surface-card:    #04202a;
  --surface-sunken:  #010e13;
  --surface-raised:  #062a37;
  --surface-inverse: #eef4f7;
  --surface-overlay: rgba(1, 10, 14, 0.70);
  --surface-hover:   rgba(34, 211, 238, 0.07);

  --border-subtle:   rgba(34, 211, 238, 0.14);
  --border-default:  rgba(34, 211, 238, 0.26);
  --border-strong:   rgba(34, 211, 238, 0.50);

  --text-strong:     #ccf5ff;
  --text-body:       #9fd8e6;
  --text-muted:      #7fb9c8;
  --text-faint:      #548a99;
  --text-on-accent:  #02141a;
  --text-inverse:    #06323d;

  --accent:          #22d3ee;
  --accent-hover:    #67e8f9;
  --accent-pressed:  #0e7490;
  --accent-subtle:   rgba(34, 211, 238, 0.10);
  --accent-subtle-border: rgba(34, 211, 238, 0.32);
  --accent-text:     #67e8f9;

  --glow-accent:     #22d3ee;

  --success:         #a3e635;
  --success-text:    #bef264;
  --success-subtle:  rgba(163, 230, 53, 0.12);
  --warning:         #fcd34d;
  --warning-text:    #fde68a;
  --warning-subtle:  rgba(252, 211, 77, 0.12);
  --danger:          #fb7185;
  --danger-text:     #fda4af;
  --danger-subtle:   rgba(251, 113, 133, 0.12);

  --focus-ring:      rgba(34, 211, 238, 0.55);
  --selection-bg:    rgba(34, 211, 238, 0.28);
```

注意 `--text-on-accent` 深色档改成了 `#02141a`(深底色)——因为主按钮是青实底,上面必须是深字。

- [ ] **Step 5: 跑测试**

```bash
cd /Users/abiu/GolandProjects/project/KiteTerm/desktop && pnpm test
```

Expected: 全部通过。`terminal.css` 尚未改,但它引用了 `colors.css` 的 `--slate-*` / `--azure-*` 原始色板(未动),所以终端配色此时仍是旧的、测试仍绿。若这里报 `未定义的变量`,说明误删了原始色板,回退。

- [ ] **Step 6: 人工看两档**

dev 窗口切深浅主题各看一遍。此时终端还是旧配色(下个任务处理),界面其余部分应已变成蓝图色。**重点看浅色档**——晒图版是推演的,若偏冷或对比过强,在此调整后再往下走。

- [ ] **Step 7: 提交**

```bash
cd /Users/abiu/GolandProjects/project/KiteTerm
git add desktop/src/theme/colors.css desktop/src/theme/effects.css
git commit -m "feat(desktop): 色彩与效果 token 换成蓝图 HUD

深色档深青黑底 + 青色边框,浅色档做成同一语言反相的白底青墨线晒图。
圆角全面降档(md 10px -> 2px),新增 --grid-size / --grid-line 供网格底用。
阴影降档:深色档退化为描边,蓝图风不用投影。"
```

---

## Task 3: 重配终端 ANSI 十六色

**Files:**
- Modify: `desktop/src/theme/terminal.css`
- Test: `desktop/src/theme/terminal.test.ts`(**只跑,不改**)

**Interfaces:**
- Consumes: 任务 2 的语义色(但终端色**不引用**语义 token,两档各自写死字面值——这是该文件既有约定,见其文件头注释)
- Produces: 无下游依赖

下列取值已用测试同款公式逐项验算过(深底 `#01171e` luminance 0.0071、浅底 `#f4f9fb` luminance 0.9395)。

- [ ] **Step 1: 替换浅色档**

```css
:root {
  --term-bg:        #f4f9fb;
  --term-fg:        #06323d;
  --term-cursor:    #0891b2;
  --term-selection: rgba(8, 145, 178, 0.20);

  --term-black:   #06323d;
  --term-red:     #be123c;
  --term-green:   #4d7c0f;
  --term-yellow:  #b45309;
  --term-blue:    #0369a1;
  --term-magenta: #7e22ce;
  --term-cyan:    #0e7490;
  --term-white:   #9fb8c0;

  --term-bright-black:   #4a7d8b;
  --term-bright-red:     #e11d48;
  --term-bright-green:   #5c920c;
  --term-bright-yellow:  #c76a05;
  --term-bright-blue:    #0284c7;
  --term-bright-magenta: #9333ea;
  --term-bright-cyan:    #0891b2;
  --term-bright-white:   #dceaef;

  --term-code-fg: #0e7490;
}
```

`bright-green` 用 `#5c920c` 而非更常见的 `#65a30d`——后者对 `#f4f9fb` 只有 2.91:1,过不了 3:1 那道关。`bright-yellow` 同理,`#d97706` 正好卡在 3.00 没有余量,改用 `#c76a05`(3.60:1)。

- [ ] **Step 2: 替换深色档**

```css
[data-theme="dark"] {
  --term-bg:        #01171e;
  --term-fg:        #ccf5ff;
  --term-cursor:    #22d3ee;
  --term-selection: rgba(34, 211, 238, 0.30);

  --term-black:   #0a2730;
  --term-red:     #ff6b81;
  --term-green:   #a3e635;
  --term-yellow:  #fcd34d;
  --term-blue:    #38bdf8;
  --term-magenta: #c084fc;
  --term-cyan:    #22d3ee;
  --term-white:   #a8cdd8;

  --term-bright-black:   #5a8896;
  --term-bright-red:     #ff9db0;
  --term-bright-green:   #c5f26a;
  --term-bright-yellow:  #fde68a;
  --term-bright-blue:    #7dd3fc;
  --term-bright-magenta: #d8b4fe;
  --term-bright-cyan:    #67e8f9;
  --term-bright-white:   #e8f7fb;

  --term-code-fg: #67e8f9;
}
```

- [ ] **Step 3: 跑契约测试**

```bash
cd /Users/abiu/GolandProjects/project/KiteTerm/desktop
node --experimental-strip-types src/theme/terminal.test.ts
```

Expected: `TERMINAL_THEME_PASS(浅色 21 项 / 深色 21 项)`

若报「对比度不足」,**改配色不改测试**:把该色朝远离本档底色的方向调,重跑。若报「两档键不一致」,说明漏了某个键。

- [ ] **Step 4: 跑全部前端测试**

```bash
cd /Users/abiu/GolandProjects/project/KiteTerm/desktop && pnpm test
```

Expected: 五个测试文件全绿。

- [ ] **Step 5: 人工看终端**

dev 窗口开一个会话(或本地终端),跑 `ls --color=always` 之类能出多色的命令,深浅两档各看一遍。重点看注释灰(`bright-black`)是否还读得出来。

- [ ] **Step 6: 提交**

```bash
cd /Users/abiu/GolandProjects/project/KiteTerm
git add desktop/src/theme/terminal.css
git commit -m "feat(desktop): 终端 ANSI 十六色换成蓝图配色

深浅两档各 21 个值重挑。terminal.test.ts 一行没改 —— 它是契约测试(解析
CSS 离线算对比度),新配色守住 ANSI >= 3:1 / fg >= 4.5:1 就自动通过。

浅色档 bright-green 没用常见的 #65a30d(对 #f4f9fb 仅 2.91:1),bright-yellow
没用 #d97706(正好 3.00 无余量),均已调深。"
```

---

## Task 4: 字体角色调整

**Files:**
- Modify: `desktop/src/theme/fonts.css`
- Modify: `desktop/src/theme/typography.css`

**Interfaces:**
- Produces: `--font-heading` 指向 mono 栈,供 `base.css` 的 `h1..h6` 与 `.screen-title` 使用

- [ ] **Step 1: 了解现状(已探明,照抄即可)**

`typography.css` 现有定义:

```css
  --font-display: "Space Grotesk", "Noto Sans SC", -apple-system, BlinkMacSystemFont, "PingFang SC", "Microsoft YaHei", sans-serif;
  --font-sans:    "Manrope", "Noto Sans SC", -apple-system, BlinkMacSystemFont, "PingFang SC", "Microsoft YaHei", sans-serif;
  --font-mono:    "JetBrains Mono", "SF Mono", ui-monospace, "Cascadia Code", Menlo, monospace;
  /* ... */
  --font-heading: var(--font-display);
  --font-body:    var(--font-sans);
```

关键:**`--font-mono` 没有 CJK 回退**。所以不能简单写 `--font-heading: var(--font-mono)`——
那样中文标题会落到系统默认等宽字体上,多数机器上难看甚至缺字。改 `--font-display` 才对。

- [ ] **Step 2: 从 fonts.css 去掉 Space Grotesk**

```css
@import url('https://fonts.googleapis.com/css2?family=Manrope:wght@400;500;600;700;800&family=JetBrains+Mono:wght@400;500;600;700&family=Noto+Sans+SC:wght@400;500;700;900&display=swap');
```

- [ ] **Step 3: 改 --font-display 为等宽打头 + 中文回退**

```css
  --font-display: "JetBrains Mono", "Noto Sans SC", ui-monospace, "SF Mono", "PingFang SC", "Microsoft YaHei", monospace;
```

英文标题走 JetBrains Mono,中文落到 Noto Sans SC(非等宽但可读)。`--font-heading` 已指向
`--font-display`,不必改。**不要**动 `--font-sans`、`--font-body` 与 `--font-mono`。

- [ ] **Step 4: 标题大写与字距**

在 `base.css` 的 `h1..h6` 规则中追加:

```css
  text-transform: uppercase;
  letter-spacing: 0.08em;
```

注意:中文不受 `text-transform` 影响,`letter-spacing` 对中文标题会略微撑开,属预期。

- [ ] **Step 5: 跑测试并人工看**

```bash
cd /Users/abiu/GolandProjects/project/KiteTerm/desktop && pnpm test
```

Expected: 全绿(字体不参与任何断言)。

dev 窗口逐屏看标题。**这是主观性最强的一步**:等宽大写标题若显得笨重,可只保留 mono 而去掉 `uppercase`,或退回无衬线标题。改动小,试错成本低。

- [ ] **Step 6: 提交**

```bash
cd /Users/abiu/GolandProjects/project/KiteTerm
git add desktop/src/theme/fonts.css desktop/src/theme/typography.css desktop/src/theme/base.css
git commit -m "feat(desktop): 标题改等宽大写,砍掉 Space Grotesk

标题/标签走 JetBrains Mono 大写 + 字距,正文保持 Manrope + Noto Sans SC
(中文等宽长文本可读性差)。顺带少一个 webfont。"
```

---

## Task 5: 网格底与角标

**Files:**
- Modify: `desktop/src/theme/components.css`(`.app-main`、`.screen`、`.card`)

**Interfaces:**
- Consumes: 任务 2 的 `--grid-size` / `--grid-line`

- [ ] **Step 1: 给 .app-main 加网格底**

找到 `.app-main` 规则(现为 `flex: 1; min-width: 0; display: flex; flex-direction: column; overflow: hidden;`),追加:

```css
.app-main {
  background-color: var(--surface-page);
  background-image:
    linear-gradient(var(--grid-line) 1px, transparent 1px),
    linear-gradient(90deg, var(--grid-line) 1px, transparent 1px);
  background-size: var(--grid-size) var(--grid-size);
}
```

用 `background-image` 而非伪元素,避免与现有 flex 布局层叠冲突。

- [ ] **Step 2: 让 .screen 透出网格**

```css
.screen { flex: 1; overflow: auto; padding: 28px 32px; background: transparent; }
```

- [ ] **Step 3: 给 .card 加角标**

```css
.card {
  background: var(--surface-card);
  border: 1px solid var(--border-subtle);
  border-radius: var(--radius-lg);
  box-shadow: var(--shadow-xs);
  position: relative;
}
.card::before,
.card::after {
  content: "";
  position: absolute;
  width: 6px;
  height: 6px;
  border-color: var(--accent);
  opacity: 0.55;
  pointer-events: none;
}
.card::before { top: -1px; left: -1px; border-top: 1px solid; border-left: 1px solid; }
.card::after { bottom: -1px; right: -1px; border-bottom: 1px solid; border-right: 1px solid; }
```

**只给 `.card` 加**,不要给 `.panel` / `.list-row` / `.badge` 也加——满屏刻线会显得廉价。

- [ ] **Step 4: 检查角标有没有被裁掉**

角标定位在 `-1px`(盒子外侧 1px)。若某处 `.card` 的父容器有 `overflow: hidden`,角标会被裁。排查:

```bash
cd /Users/abiu/GolandProjects/project/KiteTerm/desktop
grep -n 'overflow: hidden' src/theme/components.css
```

对命中的容器,若其内部直接放 `.card`,把该 `.card` 的角标改为 `top: 0; left: 0` 内嵌式,或给容器留 1px 内边距。dev 窗口逐屏确认。

- [ ] **Step 5: 跑测试**

```bash
cd /Users/abiu/GolandProjects/project/KiteTerm/desktop && pnpm test
```

Expected: 全绿。

- [ ] **Step 6: 提交**

```bash
cd /Users/abiu/GolandProjects/project/KiteTerm
git add desktop/src/theme/components.css
git commit -m "feat(desktop): 内容区加图纸网格,卡片加对角角标

网格挂在 .app-main 而非全局,侧栏与状态栏保持实色面板,层次才分得开。
角标只给 .card —— 每个盒子都上会满屏刻线,反而廉价。"
```

---

## Task 6: 组件逐类改法

**Files:**
- Modify: `desktop/src/theme/components.css`

**Interfaces:**
- Consumes: 任务 2 的语义色与圆角

- [ ] **Step 1: 主按钮改成全局唯一实心元素**

```css
.btn-primary {
  background: var(--accent);
  color: var(--text-on-accent);
  border: 1px solid var(--accent);
  font-weight: var(--weight-semibold);
}
.btn-primary:hover { background: var(--accent-hover); border-color: var(--accent-hover); box-shadow: var(--glow-sm); }
```

- [ ] **Step 2: 次级按钮改描边式**

```css
.btn-secondary,
.btn-outline {
  background: transparent;
  color: var(--accent-text);
  border: 1px solid var(--border-default);
}
.btn-secondary:hover,
.btn-outline:hover { border-color: var(--accent); color: var(--accent-hover); }
```

- [ ] **Step 3: 侧栏选中态改左竖条**

```css
.nav-item[data-active="true"] {
  background: var(--accent-subtle);
  color: var(--accent-text);
  border-left: 2px solid var(--accent);
  padding-left: 10px;   /* 原 12px,减去竖条宽度以免文字位移 */
}
.sidebar[data-collapsed="true"] .nav-item[data-active="true"] {
  border-left-width: 2px;
  padding-left: 0;
  justify-content: center;
}
```

折叠态只剩图标居中,竖条仍贴左边;`padding-left: 0` 抵消展开态的补偿值。

- [ ] **Step 4: 面板头加分隔线,标题转大写**

```css
.panel-head { border-bottom: 1px solid var(--border-subtle); }
.panel-title {
  font-family: var(--font-mono);
  font-size: 12px;
  font-weight: var(--weight-semibold);
  letter-spacing: 0.08em;
  text-transform: uppercase;
  color: var(--text-muted);
}
```

- [ ] **Step 5: 徽章改方框描边式**

```css
.badge {
  border-radius: var(--radius-xs);
  border: 1px solid currentColor;
  background: transparent;
  font-family: var(--font-mono);
  font-size: 10px;
  letter-spacing: 0.06em;
  text-transform: uppercase;
}
```

`.badge .dot` 保持 `--radius-full`(状态点仍是圆的),不要一起改方。

- [ ] **Step 6: 输入框 focus 转青**

```css
.input:focus {
  border-color: var(--accent);
  box-shadow: var(--glow-sm);
  outline: none;
}
```

- [ ] **Step 7: 状态栏数值等宽对齐**

```css
.statusbar { border-top: 1px solid var(--border-subtle); }
.statusbar .grp { font-variant-numeric: tabular-nums; }
.statusbar .grp + .grp { border-left: 1px solid var(--border-subtle); padding-left: 16px; }
```

- [ ] **Step 8: 跑测试并逐屏人工看**

```bash
cd /Users/abiu/GolandProjects/project/KiteTerm/desktop && pnpm test
```

Expected: 全绿。

dev 窗口把 10 个屏全点一遍,深浅两档都看。重点查:按钮主次是否一眼分明、侧栏折叠态竖条是否错位、徽章描边在浅色档是否太淡。

- [ ] **Step 9: 提交**

```bash
cd /Users/abiu/GolandProjects/project/KiteTerm
git add desktop/src/theme/components.css
git commit -m "feat(desktop): 组件层改成线框式

主按钮是全局唯一实心元素,其余皆描边,主次一眼分明。侧栏选中态从圆角块
改成左竖条(折叠态单独处理),徽章改方框描边,面板标题转等宽大写。"
```

---

## Task 7: `ui.tsx` 三处原语

**Files:**
- Modify: `desktop/src/components/ui.tsx`
- Modify: `desktop/src/theme/components.css`(新原语的样式)
- Read-only: `desktop/src/types.ts`(取 `Screen` 枚举顺序)

**Interfaces:**
- Consumes: `types.ts` 的 `Screen` 类型(10 项,顺序为 `terminal, hosts, transfer, forwards, certs, monitor, keys, snippets, sync, settings`)
- Produces:
  - `Readout({ label, value, unit?, tone? }): JSX.Element` — `tone?: "ok" | "warn" | "bad"`
  - `Badge` 新增 prop `frame?: boolean`
  - `ScreenHeader` 新增可选 prop `screen?: Screen`(传了才显示序号)

- [ ] **Step 1: 新增 Readout 原语**

在 `ui.tsx` 末尾追加:

```tsx
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
```

- [ ] **Step 2: Readout 的样式**

在 `components.css` 追加:

```css
.readout { display: flex; flex-direction: column; gap: 2px; }
.readout-label {
  font-family: var(--font-mono);
  font-size: 10px;
  letter-spacing: 0.1em;
  text-transform: uppercase;
  color: var(--text-faint);
}
.readout-value {
  font-family: var(--font-mono);
  font-size: 15px;
  font-variant-numeric: tabular-nums;
  color: var(--text-strong);
  display: flex;
  align-items: baseline;
  gap: 2px;
}
.readout-unit { font-size: 10px; color: var(--text-muted); }
.readout[data-tone="success"] .readout-value { color: var(--success-text); }
.readout[data-tone="warning"] .readout-value { color: var(--warning-text); }
.readout[data-tone="danger"] .readout-value { color: var(--danger-text); }
```

- [ ] **Step 3: Badge 加 frame 变体**

把 `Badge` 的签名与 className 拼接改为:

```tsx
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
```

配套样式:

```css
.badge-frame { border-radius: var(--radius-xs); }
```

(任务 6 已把 `.badge` 基础样式改成描边式,这里只需覆盖圆角。)

- [ ] **Step 4: ScreenHeader 加刻度基线与真实序号**

```tsx
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
```

`SCREEN_ORDER` 必须与 `types.ts` 的 `Screen` 枚举顺序一致。改了那边记得同步这里。

- [ ] **Step 5: 序号与刻度基线的样式**

```css
.screen-ord {
  margin-left: 10px;
  padding-left: 10px;
  border-left: 1px solid var(--border-subtle);
  color: var(--text-faint);
  font-variant-numeric: tabular-nums;
}
.screen-header { border-bottom: 1px solid var(--border-subtle); padding-bottom: 12px; }
```

- [ ] **Step 6: 类型检查**

```bash
cd /Users/abiu/GolandProjects/project/KiteTerm/desktop && pnpm exec tsc -b
```

Expected: 无错误。`screen` 与 `frame` 都是可选 prop,现有调用点不传也能编译。

- [ ] **Step 7: 跑测试**

```bash
cd /Users/abiu/GolandProjects/project/KiteTerm/desktop && pnpm test
```

Expected: 全绿。

- [ ] **Step 8: 提交**

```bash
cd /Users/abiu/GolandProjects/project/KiteTerm
git add desktop/src/components/ui.tsx desktop/src/theme/components.css
git commit -m "feat(desktop): 补 Readout 原语,ScreenHeader 加序号,Badge 加 frame

Readout 统一「标签 + 等宽数值 + 单位」的排版与 tabular-nums,替掉各屏手写
拼装对不齐的问题。ScreenHeader 的序号取自 Screen 枚举的真实位置,不是装饰。
两个新 prop 都可选,现有调用点无需改动。"
```

---

## Task 8: 批次 1、2 屏幕接入新原语

**Files:**
- Modify: `desktop/src/screens/TerminalScreen.tsx`
- Modify: `desktop/src/screens/HostsScreen.tsx`
- Modify: `desktop/src/screens/ForwardsScreen.tsx`
- Modify: `desktop/src/screens/MonitorScreen.tsx`

**Interfaces:**
- Consumes: 任务 7 的 `Readout`、`ScreenHeader` 的 `screen` prop、`Badge` 的 `frame` prop

- [ ] **Step 1: 找出手写的「标签 + 数值」结构**

```bash
cd /Users/abiu/GolandProjects/project/KiteTerm/desktop
grep -n 'stat-value\|num-readout\|stat-label' src/screens/*.tsx src/screens/*/*.tsx
```

命中的位置就是该换 `Readout` 的地方。逐个替换,**保留原有的数值计算逻辑,只换呈现**。

- [ ] **Step 2: 四个屏的 ScreenHeader 传 screen prop**

```tsx
<ScreenHeader eyebrow="..." title="..." screen="hosts" />
```

四处分别传 `"terminal"` / `"hosts"` / `"forwards"` / `"monitor"`。

- [ ] **Step 3: 状态类徽章加 frame**

主机在线状态、转发规则类型(L/R/D)、证书状态这类**状态标识**加 `frame`;计数类保持 pill。

- [ ] **Step 4: 类型检查与测试**

```bash
cd /Users/abiu/GolandProjects/project/KiteTerm/desktop
pnpm exec tsc -b && pnpm test
```

Expected: 均无错误。

- [ ] **Step 5: 人工逐屏看**

四个屏深浅两档都看。重点:`Readout` 的数值是否对齐、序号是否正确(终端 01、主机 02、转发 04、监控 06)。

- [ ] **Step 6: 提交**

```bash
cd /Users/abiu/GolandProjects/project/KiteTerm
git add desktop/src/screens
git commit -m "feat(desktop): 终端/主机/转发/监控四屏接入 HUD 原语

手写的标签+数值换成 Readout,状态类徽章改 frame 变体,页头带上真实序号。"
```

---

## Task 9: 收尾——剩余六屏与全量验证

前两层做完后,批次 3、4 的六个屏(传输、证书、密钥、片段、设置、同步)很可能已自动跟着变。本任务先看,再决定改不改。

**Files:**
- Modify: 视检查结果而定,可能为空

- [ ] **Step 1: 逐屏检查剩余六屏**

dev 窗口打开传输、证书、密钥、片段、设置、同步,深浅两档各看一遍。记录**真正出问题**的地方(元素错位、对比度不足、圆角残留),而不是"可以更好看"的地方。

- [ ] **Step 2: 只修问题,不做锦上添花**

对第 1 步记录的问题逐个修。若无问题,跳过本步——这是允许的结果。

- [ ] **Step 3: 全量测试**

```bash
cd /Users/abiu/GolandProjects/project/KiteTerm/desktop && pnpm test
cd /Users/abiu/GolandProjects/project/KiteTerm && cargo test --manifest-path desktop/src-tauri/Cargo.toml 2>&1 | tail -5
cd /Users/abiu/GolandProjects/project/KiteTerm/server-worker && pnpm test 2>&1 | tail -5
```

Expected: 前端五个文件全绿;Rust 91 passed / 2 ignored;worker 91 passed。后两者本次未触碰,出问题说明误改了不该改的文件。

- [ ] **Step 4: 确认没有新增硬编码色值**

```bash
cd /Users/abiu/GolandProjects/project/KiteTerm/desktop
grep -rnE '#[0-9a-fA-F]{6}' --include='*.tsx' src/ | wc -l          # 期望 0
grep -nE 'rgba?\(|#[0-9a-fA-F]{3,6}' src/theme/components.css | grep -v 'var(--' | wc -l   # 期望 <= 1
```

改造前基线是 tsx 0 处、components.css 1 处。超出即违反 Global Constraints,逐个收回 token。

- [ ] **Step 5: 提交(若有改动)**

```bash
cd /Users/abiu/GolandProjects/project/KiteTerm
git add desktop/src
git commit -m "fix(desktop): 蓝图 HUD 收尾,剩余六屏问题修正"
```

---

## 完成标准

- 10 个屏在深浅两档下都无元素错位、无对比度不足
- `pnpm test` 五个文件全绿,含 `TERMINAL_THEME_PASS(浅色 21 项 / 深色 21 项)`
- `cargo test` 91 passed / 2 ignored(未受影响)
- `.tsx` 硬编码色值仍为 0 处
- `terminal.test.ts` 一行未改
