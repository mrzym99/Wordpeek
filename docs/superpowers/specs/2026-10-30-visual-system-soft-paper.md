# 视觉系统统一：中性灰分层（VS Code 参考）

日期：2026-10-30
范围：全应用（划词弹窗 / 输入翻译 / 设置 / 划词小释义卡 / 截图选区），只做视觉与少量可达性结构改动，不改业务逻辑。

## 目标

1. 让弹窗看起来像「词典」而不是「文档应用」：身份来自词典本身（衬线词头 + 斜体音标 + 左 gutter 序号），不靠装饰。
2. 层次代替描边：三层中性表面（外壳/内容/抬起）、按层分配的圆角与贴身阴影，**不用降对比度换柔和**。色彩参考 VS Code：外壳（标题栏、侧栏）偏暗，内容比外壳浅一档。
3. 一套 token 覆盖浅色 + 深色，颜色不再散落在 20 处硬编码里。
4. 设置页不再拥挤：窗口加宽、左菜单收窄且字号降一级。

## 现状诊断（行号为实施时的定位锚点）

- `src/style.css:8-17`：注释写着「Notion 风格配色」，`#37352f` / `#2383e2` 整套照搬 Notion。
- 硬编码色 20 处：`#f5f7fa`（`:466,553,593`）、`#4f8cff`（`:451,573`）、`#2383e2`（`:321`）、`rgba(35,131,226,.15)`（`:396,423`）、`#22a06b`（`:246`）、`#2f9e44`（`:334`）、`#b9c6d4`（`:438`）、`#ececea`（`:275`）、`#e8e8e6`（`:277`）、`#e7f3f8`（`:222`）、`#888`（`:584`）、`#e3e2e0`（`:414`）、阴影（`:32-34`、`:841-843`）、`#34a853`（`:820`）。
- 字号 10 个值（10/11/12/12.5/13/13.5/14/15/17/18），圆角 5 种（3/4/6/8/10），均无级差。
- 对比度不足：`--text-faint #9b9a97` 白底 2.8:1、`.shot-loading #888` 3.5:1，用在提示/加载/词性标签这类正文级文字上。
- 键盘不可达：同义词 `<span onClick>`、词形 `<b onClick>`（`src/components/Popup.tsx` 渲染同义词与词形处）；`.text-btn` / `.icon-btn` / `.nav-item` 无 `:focus-visible`。
- 弹窗首行给品牌：`Wordpeek beta v0.5.6` 占 32px 高标题行（`.word-head` + `.app-title` / `.app-beta` / `.app-version`）。
- 阴影失效：`.card` 铺满窗口（`src/style.css:1-6` height:100%），光晕画在窗口外被裁，只剩一点边缘。

## Token（`src/style.css` 顶部整段替换）

**定稿是中性灰 + VS Code 蓝**。中间试过一轮「取自应用图标」的靛紫（深底 `#251D55`、放大镜 `#A1A2F9`→`#70D4F9`）与更早的青绿 `#38746C`，都被否掉了：用户要的是 VS Code 那种「外壳偏暗、内容浅一档」的中性分层，不要带色相的主题色。强调色用 VS Code 蓝（浅 `#005FB8` / 深 `#3794FF`），中性色不带任何色相。

```css
:root {
  --paper: #FFFFFF;          /* 内容面（卡片、设置右侧内容） */
  --paper-raised: #FFFFFF;   /* 抬起面（浮层、聚焦输入、选中项） */
  --wash: #F3F3F3;           /* 外壳面（设置左菜单、输入窗口底、内嵌填充块） */
  --wash-hover: #E8E8E8;
  --ink: #202020;            /* 16.3:1 */
  --ink-2: #616161;          /* 6.2:1 */
  --ink-3: #6E6E6E;          /* 5.1:1，仅限非唯一信息来源，且 ≥11px */
  --line: #E5E5E5;
  --thumb: rgba(0,0,0,.22); /* 细滚动条 */
  --accent: #005FB8;         /* VS Code 蓝，6.3:1（wash 上 5.7） */
  --on-accent: #FFFFFF;      /* 深色模式下 accent 更亮，用 #102A43 而不是白字 */
  --accent-wash: #E8F1FB;
  --danger: #A1260D;         /* 7.5:1 */

  --font-read: ui-serif, "New York", "Songti SC", Georgia, serif; /* 只给词头/音标/词形值：拉丁短词 */
  --font-ui: -apple-system, "PingFang SC", "Segoe UI", "Microsoft YaHei", sans-serif;

  --t-head: 19px;   /* 词头，--font-read 500，line-height 1.25，letter-spacing -0.01em */
  --t-lead: 15px;   /* 分区标题，--font-ui 600，l-h 1.4 */
  --t-body: 13px;   /* 释义与界面都用 --font-ui，l-h 1.7 / 1.6 */
  --t-small: 12px;  /* 音标、chip、标签、左菜单，l-h 1.5 */
  --t-micro: 11px;  /* 版本、hint，l-h 1.4（下限，不再小） */

  --r-xs: 8px;      /* 内嵌 wash 块、输入框、菜单项 */
  --r-md: 12px;     /* 按钮 */
  --r-lg: 18px;     /* 窗口卡片 */
  --r-pill: 999px;  /* badge / 同义词 / 语言切换 */

  --shadow-card: 0 0 0 1px rgba(0,0,0,.07), 0 1px 2px rgba(0,0,0,.06);
  --shadow-pop: 0 0 0 1px rgba(0,0,0,.07), 0 2px 6px rgba(0,0,0,.07),
                0 10px 28px rgba(0,0,0,.12);

  --ease: cubic-bezier(.2,.7,.3,1);
  --dur: 140ms;
}

/* 主题由 <html data-theme="light|dark"> 决定（system 在 JS 里先解析） */
:root[data-theme="dark"] {
    --paper: #1F1F1F;        /* 内容面 */
    --paper-raised: #2B2B2B; /* 抬起面 */
    --wash: #181818;         /* 外壳面 */
    --wash-hover: #2A2A2A;
    --ink: #CCCCCC;          /* 10.3:1 */
    --ink-2: #A5A5A5;        /* 6.7:1 */
    --ink-3: #8E8E8E;        /* 5.0:1 */
    --line: #3C3C3C;
    --thumb: rgba(255,255,255,.22);
    --accent: #3794FF;       /* VS Code 蓝，5.4:1 */
    --on-accent: #102A43;
    --accent-wash: #142B42;
    --danger: #F48771;       /* 6.7:1 */
    --shadow-card: 0 0 0 1px rgba(0,0,0,.5), 0 1px 2px rgba(0,0,0,.4);
    --shadow-pop: 0 0 0 1px rgba(0,0,0,.5), 0 2px 6px rgba(0,0,0,.4),
                  0 10px 28px rgba(0,0,0,.5);
}
```

只用一套强调色：成功态（复制成功 / 已保存 / 已配置源）不再用绿色 `--ok`，统一 `--accent`，少一个色相。

间距统一到 4 的倍数：4 / 8 / 12 / 16 / 20 / 24。

## 版式

### 划词弹窗（单词）

```
┌────────────────────────────────────┐
│                          对比各源   │  24px 行，ghost 按钮，无底色
│ serendipity                        │  --t-head read
│ 美 ˌserənˈdɪpəti   英 …            │  read italic --t-small --ink-3
│                                    │
│ 1  意外发现珍奇事物的本领            │  序号在 22px gutter（--ink-3），正文 --ink
│ 2  机缘巧合                         │  li 间距 4px，行高 1.7
│                                    │
│ 词形变化                             │  12px 间距 + --t-small --ink-2 标签，无 border-top
│   serendipitous  serendipitously    │
│ 同义词 n. 幸运                       │
│   ⬭luck ⬭fortune ⬭chance           │  pill，--wash 底，--accent 字
└────────────────────────────────────┘
```

- 头部去掉 `.app-title` / `.app-beta` / `.app-version`；版本号移到设置「关于」。
- header 高 32 → 24px，同步 `useAutoFitWindow("main", 67, 190)`（`src/components/Popup.tsx`）的 chrome 常量与 Rust 侧一致值（67 → 59，改动前后必须两边一起改）。
- 卡片出现：`opacity 0→1` + 上移 2px，140ms，一次性；`prefers-reduced-motion` 全关。

### 句子 / 截图模式

原文与译文各一个 `--wash` 块（`--r-xs`，12px 内距），标签 12px `--ink-2` 在框内左上，复制按钮 hover 淡入（沿用现有行为）；不画边框。语言切换用 pill 组，激活项 `--accent-wash` 底 + `--accent` 字。

### 输入翻译

同弹窗；`.input-layout` 底色 `--paper`，内容框 `--wash`。

### 划词小释义卡（`.pop-card`）

`--paper-raised` 底、`--r-md`、`--shadow-pop`、边框去掉（改由 ring 提供），宽 252 保持。

### 截图选区

蒙层保持 `rgba(0,0,0,.35)`；选区边框 `#4f8cff` → `#3794FF`（深色蒙层上够亮），`inset 0 0 0 1.5px` 同色。

### 设置页（475 → 600 宽，解决拥挤）

- 窗口：`src-tauri/src/main.rs:255` `.inner_size(500.0, 360.0)` → `.inner_size(600.0, 440.0)`，`resizable(false)` 不变；无其它尺寸耦合（已核对 `src-tauri/src/`）。
- 左菜单 `.settings-nav`：`128px → 104px`（`src/style.css:260`），padding `14px 8px`，菜单项 `--t-small`（13 → 12px）、padding `5px 8px`、圆角 `--r-xs`、`margin-bottom: 2px`。
- 选中项：3px `--accent` 左竖条 + `--ink` 600 + `--paper-raised` 底（外壳面上的抬起项，等价 VS Code 选中列表项）；hover 用 `--wash-hover`。
  （原计划用浅黄色 `--mark` 做竖条，实测与纸底对比过低，3px 竖条像脏点；改版后 `--mark` / `--ok` 两个色都删了，「关于」的 beta 徽章改用 `--accent-wash` + `--accent`。）
- 右内容 `.settings-content`：padding `20px 24px 16px`，内容列 `max-width: 440px` 左对齐（表单不拉满全宽）。
- 字段 `.field` margin-bottom 10 → 12；`.group-heading` margin `16px 0 8px`；`.section-heading` 用 `--t-lead`。
- 子 tab `.sub-tab`：13px，激活用 `--accent` 文字 + 2px `--accent` 下边线（现 `#2383e2` 换 token）。
- 底部保存按钮：`--accent` 底 + `#FFFFFF` 字（5.2:1）、`--r-md`、hover 加深 6%，禁用态 `--wash` + `--ink-3`。

## 五条原则

1. 柔和靠层次（`--paper` / `--wash` / `--paper-raised` + 一条 `--line`），一屏最多一条可见分隔线。
2. 圆角按层级给，不给所有元素同一圆角；不加渐变、不加发光、不做毛玻璃、不做拟物双层内阴影。
3. 阴影只给浮动层（窗口卡片、划词小卡）；其余用 `--wash` 区分。
4. 文字对比度地板 4.5:1（12px 及以上）；`--ink-3` 只到 4.6，不承载唯一信息。
5. 结构装置只留左 gutter 序号；不上全大写 eyebrow、不用 `A · B · C` 拼接、不做衬线大标题装饰。

## 实施清单

1. `src/style.css:8-17` 替换为 token 块（浅 + 深）。
2. 把上述 20 处硬编码色换成 token。
3. 字号 10 → 5 级、圆角 5 → 4 级、间距对齐 4 的倍数；`.senses li::before` 序号进 22px gutter（`text-align: right` 对齐）。
4. `src/components/Popup.tsx`：删掉 `.app-title` 块（版本号已在设置「关于」）；header 高度保持 24px + 8px margin，**正好等于原来的 32**，所以 autofit chrome 常量（67）与 Rust 侧都不用动。
5. 设置窗口尺寸（`src-tauri/src/main.rs:255`）+ `.settings-nav` / `.settings-content` / `.nav-item` 三项重写。
6. 结构可达性（少量 JSX）：同义词 `.syno`、词形 `.word-form b` 换 `<button>`；`.text-btn` / `.icon-btn` / `.nav-item` / `.config-menu-item` 加 `:focus-visible`（2px `--accent` + 2px offset）。
7. 版本号从弹窗搬到设置「关于」（无需新增：`SettingsPage.tsx:555` 已在显示 `v{current_version}`）。

## 实施记录（2026-10-30，已落地）

- `src/style.css` 整文件重写；`src/components/Popup.tsx`（删标题行、syno/词形换 button）；`src/components/SettingsPage.tsx`（nav-item / config-menu-item 换 button）；`src-tauri/src/main.rs:255` 尺寸 600×440。
- 偏离方案一处：**释义正文不做衬线**。首版按方案给 `.senses` 上了 `--font-read`，实际渲染 13px 宋体中文笔画发虚（「表示问候，惊奇或唤起注意时的用语」明显可读性下降），且与「柔和 + 易读」冲突。改为衬线只用于词头（`.word-big`）、音标（`.phonetic`）、词形值（`.link-btn`）、小卡词头（`.pop-word`）——这也正是词典惯例（词头衬线、正文无衬线）。Windows 无 `ui-serif` 时回落 Georgia 的风险随之只影响拉丁短词，可接受。
- 新增两个方案外的 token：`--on-accent`（深色下 `--accent` 很亮，白字会失效）与 `--thumb`（细滚动条）。
- 配色改了两次，第 3 版定稿：首版青绿 `#38746C`（用户：绿色不好看）→ 靛紫/长春花蓝（从 `app-icon-1024.png` 取色）→ **定稿中性灰 + VS Code 蓝**（用户：「不用紫色主题了，就参考 vscode 那种，内容部分选用比头部颜色浅一点的配色」）。`--mark` / `--ok` 两个色相在两轮里都已删掉。
- 目视验收：单词卡 / 输入翻译卡 / 设置页 在 380px 与 600×440 两种宽度、浅色与深色各过一遍（`shots/` 下临时截图，未入库）。
- 深色改为可配（10-30 追加）：`@media (prefers-color-scheme: dark)` 改成 `:root[data-theme="dark"]`，加 `color-scheme`；`<html data-theme>` 由 `src/lib/theme.ts` 统一落地（`main.tsx` 里所有窗口共用一份），设置 → 外观 三档（跟随系统/亮色/暗色）落盘到 `AppConfig.theme`（后端 `save_theme`，值非法即拒）。首屏不闪浅色靠 `index.html` 里读 localStorage 缓存的内联脚本。
- 划词卡片左上补回品牌字 `Wordpeek` + beta（版本号仍只在设置 → 关于），占标题行左侧，不增高（`--t-small`/600/`--ink-2`）。
- `.input-layout` 加 `min-height: 100vh`：输入窗口比内容高时，多出的部分也上色，不再露出原生窗口白底（深色下最刺眼）。
- 卡片头 `.word-head` **试过铺 `--wash` 当标题栏，已按用户要求撤回**：划词卡片是一个无标题栏的浮窗，头部没有关闭按钮，也不该有标题栏色。现在头部完全透明、与卡片同色，只有「设置」「输入翻译」这类带系统标题栏的窗口才需要谈头部配色（系统原生标题栏 CSS 改不到，要么自绘 Overlay，见风险）。
- 参考 VS Code 的分层重排了一次表面（用户给了深/浅两张编辑器截图）：三层语义定为「卡片面 `--paper` < 外壳面 `--wash` < 内容/抬起面 `--paper-raised`」，深色下三档从暗到亮、浅色下从亮到暗，两套方向一致地随主题镜像。落地：`.settings-nav` 铺 `--wash`、`.settings-content` 铺 `--paper-raised`（侧栏/内容两色分开，VS Code 的核心手感），`.nav-item.active` 用 `--paper-raised` + 3px 竖条，`.input-layout` 外壳 `--wash` 让输入卡片浮起来，`--line` 从半透明改成实色（深 `#34334F` / 浅 `#E7E7EF`）。

## 验收

- 五张截图（浅色/深色各一次）：单词卡（短词、长词换行）、句子卡、截图结果、设置页、输入翻译。
- 对比度自查：任何 ≤13px 文字不低于 4.5:1。
- 键盘走查：Tab 能到「对比各源」、同义词、词形、语言切换、设置左菜单，焦点环可见。
- 高亮元素不超过 2 处可见（`--accent` 文字 + 左菜单当前项的 `--accent` 竖条）。

## 风险

- `ui-serif` 在 macOS WKWebView 解析到 New York；Windows WebView2 无 `ui-serif` → 回落 Georgia。衬线已收窄到词头/音标/词形值（拉丁短词），中文正文一律无衬线，所以「中文落宋体偏细」的风险已消除。
- New York 对 IPA 符号的覆盖率需实测（音标必须能显示 `ˌ ə ˈ ɪ ʃ θ ŋ`）。
- 深色 token 必须在浅色与深色宿主应用上各测一次（弹窗浮在别的应用上）。
- 深色模式下 `--shadow-*` 与透明窗口叠加，边缘可能过重，需实机微调透明度。
- 「设置」「输入翻译」用的是系统原生标题栏（`tauri.conf.json` 里只有划词主窗口和截图窗口 `decorations: false`），CSS 碰不到它的颜色。要让它们和内容同色只能走 `TitleBarStyle::Overlay` + `hidden_title`，把页面画到标题栏下面（顶部留 ~28px 给交通灯，`data-tauri-drag-region` 负责拖拽），这是 Rust 侧改动，尚未做。
- 卡片铺满窗口，柔和感不能靠外扩光晕；若日后想要更松的浮起感，需要给 `.card` 加外边距并同步改 autofit 的 chrome 常量（前端 + Rust 两处），不在本次范围。
