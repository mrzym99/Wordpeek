# Wordpeek 架构重新设计（划词技术方案）

- 日期：2026-11-06
- 状态：已按 `docs/superpowers/refactor.md` 要求完成设计（需求确认：交付设计文档不动代码；pin、剪贴板兜底开关等新功能纳入设计）
- 性质：工程方案，不含实现代码；所有现状引用均带 file:line 锚点（以 2026-11-06 main 分支为准，迁移落位后需同步更新）

## 目标（refactor.md 原文归纳）

1. Windows / macOS 都支持监听用户划词
2. 快捷键监听读不到内容时，可访问剪贴板兜底——**默认不访问**；判断用户是否真的划词
3. 划词图标自动出现；点击图标或双击选词自动出卡片，此行为**禁止访问剪贴板**
4. 卡片点外部自动消失、鼠标在卡片内不消失；右上角 pin 图标支持钉住不消失

约束：React 只做 UI；Tauri 负责窗口/托盘/权限引导/IPC；原生能力全在 Rust。Windows 优先 UI Automation，macOS 优先 Accessibility API。全局鼠标/键盘输入只用于判断手势，不用于猜文本。

---

## 0. 现状诊断（设计的事实基础）

### 0.1 结构现状

Rust 侧约 5980 行。main.rs 1551 行是「事件驱动 + 静态原子标志」的编排大杂烩，五个领域互相纠缠：

| 领域 | 现状位置 | 问题 |
|---|---|---|
| 划词兜底管线 | main.rs L46-133 `grab_selection` | 剪贴板备份/序号/背刺 watch 细节与编排混杂 |
| 纯几何工具 | main.rs L136-238 | 与领域无关，纯粹放错地方（6 个单测 L1497-1550 随行） |
| 窗口 spawn | main.rs L298-378 | `spawn_or_show_window` 与托盘/齿轮入口耦合 |
| 更新器 | main.rs L380-486 | 独立领域，最易拆 |
| 图标鼠标链路 | main.rs L518-830 | **核心混乱源**：11 个静态原子/Mutex（L542-557）+ worker 线程 + 探测线程，会话状态散落三处 |
| 截图编排 | main.rs L841-1132 | 独立但长 |
| main() setup | main.rs L1216-1456 | DWM 主题补刀 L1387-1454 必须留在 run loop（「振荡事件风暴」注释 L1408-1411，**不可改 set_theme**） |

结构性问题：**没有「划词会话」概念**。`SELECTION_GEN`（L553）、`GRAB_IN_PROGRESS`（L26）、`PROBE_INTENT`（L551）、`ICON_CLICKED_AT`（L542）本质都是同一会话生命周期的碎片；前端 Popup 又用 `selSeq`/`shownAt`/`everFocused`（Popup/index.tsx L173-195）复刻了一套。新增 pin、同文本去重、兜底开关时状态会继续碎片化。

translate.rs 1362 行：5 个翻译源各自手写请求+解析（bing L200 / youdaoai L236 / youdao jsonapi 仅英→中 L581 / baidu MD5 L675 / tencent TC3 L752），match 分发在 `try_translate`（L862-880），`translate_text` 按 build_order 逐源 15s 超时降级（L886-914）；9 个 save_* 命令走 persist 模板（L383）；诊断埋点遗留（`diag()` L163、tokio 心跳 main.rs L1276）。

已探明的关键事实：
- 热键路径 `handle_hotkey_trigger`（main.rs L816-838）**直接走 grab_selection 碰剪贴板，从未先试 UIA/AX** —— 与「默认不访问剪贴板」目标冲突，是本设计最重要的行为修正点
- 卡片 pin 功能完全不存在
- `SelectionRead` 四态枚举（Text/NoTextPattern/NoSelection/Error）目前是 windows.rs 私有实现（L538），macOS `focused_selection` 只返回 `Option<String>`，两平台签名不对齐
- 版本号三处失同步：Cargo.toml `1.0.8` vs tauri.conf.json/package.json `2.1.1`
- 测试：Rust 49 个（几何/手势/build_order/OCR 预处理/剪贴板），前端仅 text.test.ts；CI 只有 release.yml（windows-latest、tag 触发），无测试流水线、无 macOS 构建任务

### 0.2 已有且正确、保留形式化的机制

- 手势判定 `gesture_selects`（main.rs L578-593，纯函数有单测）：位移 ≥6px 或连击，普通单击不探测
- 代次防过期 `SELECTION_GEN`（L553）：探测完成时校验，期间有新按下即丢弃
- 图标路径铁律：绝不碰剪贴板、不注入按键（L775-778 注释），UIA/AX 读不到就不弹
- `SelectionRead` 四态（windows.rs L538-547）：区分「这次没选中」与「应用不支持」；hopeless 拉黑带 10min TTL（platform/mod.rs L226-249）
- 钩子边界：回调只做廉价判断发 `sync_channel(1)` 信号（L678），耗时工作全在 worker；Windows 钩子看门狗（platform/windows.rs，HookGuard 所有权交接 + 静默摘除自动重装）
- 卡片收起轮询：show 即启动、`everFocused` 判别 blur 语义 / 全局鼠标命中语义、`last_mouse_down` 命令（main.rs L566-570）

---

## 1. 总体架构

### 1.1 目标分层

```
src-tauri/src/
├── main.rs            ~200 行薄壳：builder 装配、invoke_handler 汇总、RunEvent 主题处理
│                      （DWM 补刀 L1387-1454 原样保留在此，禁止迁移）
├── geometry.rs        纯几何函数整体迁出（fit_height/clamp_*/to_physical/monitor_rect_at/
│                      viewport_inset，连同 6 个单测）
├── windows/mod.rs     spawn_or_show_window / open_settings / open_input / center_window / fit_window
├── tray.rs            托盘菜单构建 + on_menu_event（main.rs L1320-1345）
├── hotkeys.rs         parse_hotkey / parse_code / validate_hotkeys / register_hotkeys（L488-535, L1142-1193）
├── updater.rs         UpdateState / check_update / install_update（L380-486）
├── screenshot.rs      ScreenshotState / handle_screenshot_trigger / finish / cancel（L841-1132）
├── selection/         划词会话领域（见 §3）
│   ├── mod.rs         SessionManager：状态机唯一所有者
│   ├── gesture.rs     鼠标手势判定（gesture_selects 等纯函数迁入）
│   ├── probe.rs       focused_selection 探测编排 + hopeless 拉黑
│   ├── fallback.rs    grab_selection 剪贴板兜底链（main.rs L46-133 + platform/mod.rs L149-224 合并）
│   └── normalize.rs   格式清洗纯函数（新增，见 §3.5）
├── translate/         拆分见 §1.3
├── ocr.rs / capture.rs 保持不变
└── platform/          保持 cfg 门面，接口小幅扩展（见 §2）
```

### 1.2 拆分收益判定（做与不做）

**做（高收益）**：
- `geometry.rs`：纯函数、有测试、零依赖，纯搬运零风险，main.rs 立减 100 行
- `selection/`：收益不是行数而是**状态归一**——11 个静态变量收进 SessionManager，是 pin、同文本去重、兜底开关三个新功能的前置条件
- `updater.rs` / `screenshot.rs` / `tray.rs` / `hotkeys.rs`：领域边界清晰，机械搬运
- translate trait 化：加一个翻译源从改 3 处（分发 match L862、build_order、前端 SourceCompare）降到改 1 处

**不做（过度工程，单人维护 + 单人项目节奏）**：
- 事件总线 / actor 框架。现有 `sync_channel(1)` + worker 线程模式简单且手势判定有单测，保留
- 窗口管理 trait（`trait AppWindow`）：5 个窗口行为差异大，trait 只会逼出 empty impl
- save_* 命令泛化：persist 模板已收敛（translate.rs L383），前端各 tab 类型化调用是优点不是债
- 前端状态库（zustand 等）：单窗口内 useState 足够，跨窗口已有 config-changed 事件
- objc2 / COM 安全封装库迁移：现有 unsafe 代码工作正常且注释充分（UIA 懒建树重试、hopeless 拉黑等针对性 workaround 在封装库里更难表达），迁移是纯风险

### 1.3 translate.rs 拆分

```
translate/
├── mod.rs        WordInfo / detect_to / AppConfig / load / persist / 命令层
├── sources/
│   ├── mod.rs    trait TranslateSource { const ID; fn supports(&self, from, to) -> bool;
│   │             async fn translate(&self, text, to, cfg: &AppConfig) -> Result<WordInfo> }
│   │             + 静态注册表：const SOURCES: [(&str, &dyn TranslateSource); N]
│   ├── bing.rs / youdao_ai.rs / youdao_dict.rs / baidu.rs / tencent.rs
└── pipeline.rs   translate_text（build_order 降级 + 15s 超时）与 try_translate
```

决策理由：
- **静态数组而非动态 registry**：5 个源编译期已知，数组遍历即实现分发与降级；`Box<dyn>` 池、`inventory`、插件化都是过度设计
- API key 配置（BaiduConfig/TencentConfig）经 `&AppConfig` 引用传入 trait 方法，不塞构造器，避免配置生命周期问题
- 「能力声明」用 `supports()` 表达（youdao jsonapi 仅英→中，L581），pipeline 统一过滤，删掉散落的硬编码判断

---

## 2. 平台分层设计

### 2.1 门面原则

platform/mod.rs 的 cfg 双分发门面（L15-147）是正确设计，保持。扩展两条接口：

**（1）`SelectionRead` 四态提升为门面公共枚举，两平台统一签名**：

```
enum SelectionRead { Text(String), NoTextPattern, NoSelection, Error }
fn focused_selection() -> SelectionRead
```

这是状态机 Probe 节点的前提：`NoSelection`（这次没选中，正常）必须区别于 `NoTextPattern`（应用不支持，可拉黑）。macOS 侧补齐 AXError 映射：`kAXErrorNoAttribute` / `kAXErrorAttributeUnsupported` → NoTextPattern；属性读到但为空 → NoSelection；其他 → Error（四态语义已在 macos.rs 的 `selection_in` 落地，仅需把私有枚举提升并统一错误码映射）。

**（2）`foreground_context()` 新增**：一次调用返回 `(pid, 进程名, 是否老控制台, 是否管理员进程, 是否密码框焦点, 是否 Secure Input)`，替代 grab 与 probe 各自零散查询（foreground_app main.rs L59、clipboard_fallback_allowed windows.rs L519、foreground_pid L503）。用途见 §4：密码框 / Secure Input 时**直接放弃**，不走兜底注入。

### 2.2 全局输入钩子边界（铁律）

现有结构已符合「钩子只判断手势」，形式化为三条不变式，写进模块文档：

1. **钩子回调内禁止**：UIA/AX 调用、剪贴板访问、窗口操作、阻塞锁竞争。只允许：读静态原子、GetCursorPos 等价物、`try_send`
2. **钩子数据只回答两个问题**：是否构成选词手势（gesture.rs 纯函数）；光标相对窗口矩形的几何关系（cursor_on_icon_rect main.rs L660）
3. **耗时工作全在 worker/独立线程**（UIA 360ms 重试、grab 900ms 轮询），经代次（gen）防过期结果

HookGuard 看门狗（windows.rs）保留。macOS 事件 tap 被系统禁用时目前无对等机制，列为补齐项（R7）：tap 回调停止触发的计时检测 → 重装。

### 2.3 平台实现要点

**Windows**：UIA TextPattern 读选区（windows.rs L453-501，Chromium 懒建树 120/240/360ms 递增重试）。UIA CacheRequest/TreeWalker 缓存仅作为远期性能优化方向，不进本次重构。

**macOS**：AX API 读 kAXSelectedTextAttribute。新增两个检测：
- **Secure Input**：`SecIsSecureInputProcess` 轮询，密码框/登录窗开启期间图标路径不弹、热键路径不注入 Cmd+C（注入会被系统吞且可能触发安全告警）。UI 不提示（用户自己也选不了），仅诊断日志
- 辅助功能权限：现有引导流程保留（alert_permission + 改权限必须换进程的说明），权限状态变化走「热键触发失败时被动检查」（L832 已如此）

---

## 3. 状态机

### 3.1 会话定义与归属

新建 `selection::SessionManager`（Tauri managed state），**划词会话唯一所有者**。现状 11 个静态变量 + 前端会话碎片收敛为：

```rust
struct Session {
    state: State,              // Idle / Gesture / Probing / IconShown / CardShown
    gen: u64,                  // 替代 SELECTION_GEN
    text: Option<String>,      // 替代 IconSelection
    shown_at: u64,             // 下发前端，替代前端 shownAt
    last_text: Option<String>, // 同文本去重
    last_shown_ms: u64,        // 同文本去重时间窗
    pinned: bool,              // pin 状态归 Rust，前端只发意图（§5.2）
}
```

前端不再自持会话状态（`everFocused` 是窗口焦点事实，保留在前端；其余由 Rust 事件下发）。

### 3.2 图标路径（鼠标手势）

```
Idle   --down-->                Gesture       # 钩子记 down 坐标；光标在自家卡片内则不启动新手势（划词中不关卡片）
Gesture --up 且 gesture_selects--> Probing     # worker 判手势（纯函数已有单测）
Gesture --up 且非手势-->          Idle         # 误触过滤：普通单击不探测
Probing --Text 且 gen 未过期且光标未飘远--> IconShown
Probing --NoTextPattern/NoSelection/超期/飘远(>400px)--> Idle
Probing --NoTextPattern-->       (mark hopeless，TTL 10min)
IconShown --down 在图标外 / 飘远 48px-->       Idle + hide
IconShown --点击图标-->          CardShown     # translate_selection_icon，绝不碰剪贴板
IconShown/Probing --双击同点（第二次 up 仍是手势）--> CardShown   # 直连路径，跳过图标显示
```

### 3.3 热键路径与合流点

```
Idle --hotkey-->                 Probing(热键模式)
Probing --focused_selection()==Text-->         CardShown          # 新增：UIA/AX 先试
Probing --读不到 且 clipboard_fallback 开 且 平台闸通过--> FallbackGrubbing --> CardShown
Probing --读不到 且 开关关-->                    Idle（静默，剪贴板零访问）
FallbackGrubbing --GRAB_IN_PROGRESS 冲突-->     丢弃本次（FlagGuard L26 已有）
```

**关键行为修正**：现状热键直接 grab_selection 碰剪贴板（L816-838）。目标热键路径 = probe 先行 → 失败才按配置兜底。与「默认不访问剪贴板」对齐，UIA 命中时零剪贴板污染。行为变化的灰度见 R2。

**合流点**：图标路径与热键路径最终都走 `SessionManager::show_card(text, x, y, source)`，内部统一：格式清洗（normalize）→ 同文本去重 → `clamp_to_screen` 钳位 → emit `selection`。source 字段（Icon / Hotkey / HotkeyFallback）随事件下发，供前端与诊断区分；payload 新字段全部 optional，旧前端容忍（R3）。

### 3.4 去抖与去重归属

| 机制 | 归属 | 现状基础 |
|---|---|---|
| 手势判定（位移≥6px / 连击，系统双击间隔 250-1000ms 夹取） | gesture.rs 纯函数，up 时刻判定 | gesture_selects L578 |
| 探测去抖（点图标后 400ms 内不探测） | SessionManager 读 ICON_CLICKED_AT | L746 |
| 代次防过期 | gen 比对，Probe 完成时校验 | SELECTION_GEN L553 |
| **同文本去重（新增）** | SessionManager：show_card 时 text 与 last_text 相同 且 距上次 show < 1500ms 且卡片仍可见 → 忽略（防双击后三击、抖动重复触发） | 无，需新建 |
| **格式清洗（新增）** | normalize.rs 纯函数，show_card 前 | 无，需新建 |
| 连按热键去抖 | GRAB_IN_PROGRESS 互斥保留 | L26 |
| 飘远丢弃 | 探测完成时光标离松手点 >400px 丢弃 | L783 |

### 3.5 格式清洗规则（normalize.rs）

纯函数 `normalize_selection(raw: &str) -> Option<String>`，返回 None = 「不像真的划词」（拦截空壳，呼应目标 2「判断用户是否真的划词」）：

- 全文 trim；全空白 → None
- 折叠硬换行（PDF/终端复制特征）：单段落内 `\n` → 空格；连续 2 个以上 `\n` 保留为段落分隔
- 折叠 3 个以上连续空格 → 1 个；全角空格规整
- 连字符断词合并（`some-\nthing` → something），默认开
- 长度上限 5000 字符截断（防拖选整页）
- 每条规则独立成 fn 便于单测（对齐现有 watch_decision 测试风格）

---

## 4. 权限与失败降级

### 4.1 剪贴板兜底开关

- AppConfig 加字段 `clipboard_fallback: bool`，`#[serde(default)]` 默认 **false**。沿用 `selection_icon` 的布尔 + save 命令模板（translate.rs L101-104, L544-568），新增 `save_clipboard_fallback` 命令，Settings OtherTab 加开关
- **默认关的语义**：false 时热键路径 UIA/AX 读不到 → 什么都不发生，剪贴板全程零访问（连备份读都不做）。true 时才进 grab_selection 链
- 现有 `clipboard_fallback_allowed()`（windows.rs L519，老控制台排除）是**独立于开关的第二道闸**。两层语义分开：开关 = 用户授权；闸 = 技术安全。生效条件：开关开 ∧ 平台闸通过
- 设置文案须说明代价：「关闭后，部分应用（终端、自绘界面）中快捷键翻译将不可用；开启后 Wordpeek 会临时读写您的剪贴板并自动恢复」

### 4.2 降级链总表

| 层级 | Windows | macOS |
|---|---|---|
| 图标路径读不到选区 | 不弹图标（铁律保留），hopeless 拉黑 10min | 同 |
| 热键路径 UIA 读不到 | 开关开→grab_selection；关→静默 | 同；AX 未授权→设置页引导（L832 已有） |
| grab 备份读不到 | 整轮放弃，不动剪贴板（L66-72 已有） | 同 |
| 鼠标钩子装不上 | 图标不可用，热键仍可用（L714-717 已有） | 同 |
| Secure Input | 不适用 | 跳过注入与探测，仅诊断日志 |
| 密码框焦点 | probe 判 IsPassword → 放弃，**绝不**注入兜底 | Secure Input 隐含覆盖 |
| 屏幕录制（截图） | 恒可用 | request_screen_capture 引导（L907-917 已有） |

### 4.3 限制场景清单

| 场景 | 限制机理 | 设计动作 |
|---|---|---|
| 管理员权限目标应用（Win） | UIA 客户端完整性低于目标进程被拒（UIPI） | foreground_context 检测目标进程 elevation（OpenProcess+GetTokenInformation），读不到时诊断日志标记 elevated；设置页提供「以管理员身份重启」入口。不自动弹 UAC（打扰） |
| 安全桌面（UAC 确认框） | 桌面切换，钩子收不到事件 | 无需动作，文档说明 |
| 锁屏 | 同上，进程存活但无输入事件 | 无动作 |
| 全屏游戏 / DirectX | 独占全屏不经 GDI 窗口管理，UIA 无 TextPattern | hopeless 拉黑自然覆盖；文档说明 |
| 终端 | 老控制台 Ctrl+C = 中断（L519 已排除）；多数终端无 TextPattern | 已覆盖（拉黑 + 老控制台闸） |
| 自绘应用（部分 Electron/Qt） | 无 UIA TextPattern / AX 属性 | hopeless 拉黑已覆盖 |
| 密码框 | Win：UIA 拒绝返回密码明文；mac：Secure Input | foreground_context 加 is_password 探查（UIA IsPasswordProperty），是则热键路径直接放弃——**宁可不翻译，绝不在密码场景注入** |

---

## 5. 卡片显隐策略

### 5.1 两种消失模式的统一形式化

现有方案（Popup/index.tsx L165-267 + last_mouse_down）已经是正确设计，本轮**不做架构迁移**（搬 Rust 需新增 IPC 心跳，收益低），只做形式化 + hook 抽取：

```
可见期间每 TICK(300ms) 求值谓词 dismiss?：
  dismiss? ≡ visible
           ∧ ¬pinned                                # pin 短路一切收起（目标 4）
           ∧ ¬cursor_in_card                        # 鼠标在卡片内永不消失
           ∧ ( everFocused ∧ ¬focused               # 模式一：可聚焦窗口，blur 语义
             ∨ ¬everFocused ∧ external_click_after_show )  # 模式二：无焦点窗口（mac Accessory
                                                    # makeKey 失灵），全局鼠标命中语义

external_click_after_show ≡ ∃(x,y,t) = last_mouse_down:
                                t > shown_at ∧ (x,y) ∉ card_rect
```

关键性质（现状已满足，作为回归基线写进文档）：
1. 轮询自 show 即启动，不依赖 blur 事件送达（mac makeKey 失灵时 blur 通道整个不存在）
2. everFocused 判别两模式，一个谓词统一两种窗口形态
3. 入场豁免：只认 t > shown_at 的按下，选词手势那一下不算「点外部」
4. 透明边距豁免：命中窗口矩形但不在 .card 内 → 交 DOM onDocMouseDown 判断，不误收
5. 每次求值幂等（isFocused/last_mouse_down 均无副作用查询），轮询丢了下轮补上

落位：谓词抽成纯函数进现有 node 测试，逻辑抽 `hooks/useCardDismiss.ts`（含 pinned 参数）。

### 5.2 pin 生命周期

- **UI**：卡片右上角 pin 图标（与关闭按钮并列），点击 `invoke("set_card_pinned", { pinned })`，图标切换斜插针/直针态
- **归属**：pinned 存于 Rust SessionManager（单一事实源），前端本地镜像用于渲染。收起判定有两处执行者（前端轮询 + Rust 新会话开始），单一事实源避免竞态
- **pin 后行为**：
  1. dismiss? 短路为 false——blur、点外部、ESC 均不再收起
  2. **无自动超时**。用户钉住的动机不可预测（对照翻译、抄写），超时违背直觉
  3. 解 pin 途径：再点 pin 图标 / 点卡片关闭按钮 / 新的划词会话开始
  4. **新会话与 pin**（最易出 bug 的边界）：用户划了新词且卡片 pinned → 新 selection 事件到达即视为放弃旧会话：解 pin + 复用同一窗口更新内容（前端 selSeq 已处理在途竞态），不新开窗口
  5. **pin 与同文本去重**：pinned 时同文本新划词不刷新卡片（用户可能正对照它），不同文本才替换
- **截图会话交互**：handle_screenshot_trigger 藏 main 窗口（L920）时 pinned 卡片一并藏起，结束按 main_was_visible 恢复（ScreenshotState 已有字段），pin 状态保留

### 5.3 其余交互细节

| 细节 | 设计 | 现状基础 |
|---|---|---|
| 显示后不抢焦 | 卡片 show 后尝试取焦（blur 模式需要）；失败（mac makeKey）自动落入模式二；icon 窗口永不 set_focus | L309-313 |
| 划词中不关卡片 | 钩子 Down 时光标在 main 窗口矩形内 → 不启动新手势、不收图标不收卡 | probe 已排除自家窗口，扩展到「不收卡」 |
| 图标防误弹 | 手势判定 + 飘远丢弃 + 自家窗口排除 + 点图标后 400ms 静默 | L573-584, L783, L746 |
| 卡片弹出位置 | 松手点 +12,+16 偏移，clamp_to_screen 钳入工作区 | L810, L825 |
| 连续划词竞态 | selSeq 代次，旧处理器醒来不覆盖新 show | index.tsx L173, L278 |

---

## 6. 推荐库 / 技术方向

**Rust 侧（全部已有依赖，不新增）**：
- Windows：`windows` crate（UIA `IUIAutomation`/`IUIAutomationTextPattern`；低级钩子 `SetWindowsHookExW`）。评估过 `uiautomation` 封装 crate：不引入，理由见 §1.2
- macOS：现有 objc runtime 绑定（AX/CGEventTap）维持；objc2 生态迁移列远期
- 剪贴板：自研 win_clipboard.rs（序号检测 + 备份恢复）+ NSPasteboard 保持（arboard 仅用于 copy_text，职责不同）
- 并发：parking_lot + tokio + reqwest 保持

**前端**：不新增依赖。结构改动仅 useCardDismiss hook 抽取；谓词纯函数并入现有 node:test（text.test.ts 模式），不引 vitest

**测试与 CI**：
- 新增 `.github/workflows/ci.yml`：windows job（cargo test 全量 + tsc + npm test）+ macos-14 job（cargo test + npm test，**不做构建/签名**，那是 release.yml 的事）。触发 push main + PR。现有 49 个 Rust 测试全部平台无关纯函数，无需真窗口
- UIA/AX 真实交互测试不进 CI（无法自动化系统权限场景），进手测矩阵（§7.2）

**版本同步**：tauri.conf.json 为唯一事实源（现 Cargo.toml 1.0.8 失同步）；迁移第一步手改 Cargo.toml 对齐，CI 加版本一致性校验（三文件比对）。updater 显示版本改读 build.rs 生成的常量而非 `app.package_info()`（现读 Cargo.toml，main.rs L393）

**诊断埋点清理**：删 tokio 心跳（main.rs L1276「破案后删」）、`translate::diag()`（translate.rs L163）及调用点。panic 钩子落盘（main.rs L1268-1274）**保留**——生产诊断正道

---

## 7. 风险清单与测试矩阵

### 7.1 风险清单

| # | 风险 | 等级 | 缓解 |
|---|---|---|---|
| R1 | 拆分过程行为回归 | 高 | 搬迁一律「纯移动不改逻辑」；geometry 单测整体迁移守门；每步独立 commit 可回滚 |
| R2 | 热键路径加 UIA 先试改变现网行为 | 高 | 开关关 + UIA 读不到 = 静默，与现状「兜底关」一致；先发「开关默认关」版本观察 |
| R3 | selection 事件 payload 加 source/session 字段前端不同步 | 中 | 字段全 optional 旧前端容忍；前后端同 PR 改 |
| R4 | pin × mac makeKey 失灵组合：pinned 但焦点未立起，按钮收不到点击 | 中 | pin/关闭按钮可点不依赖窗口焦点（onDocMouseDown 是 DOM 事件），无碍 |
| R5 | DWM 主题补刀被误重构 | 中 | 明文规定：RunEvent 主题分支（L1387-1454）原样保留 main.rs，禁止 set_theme（振荡事件风暴 L1408-1411） |
| R6 | 热键路径 UIA 探测增加延迟（Chromium 360ms）才进兜底 | 低 | 可接受：命中省剪贴板往返；未命中多 ~400ms 换「默认不碰剪贴板」的确定性 |
| R7 | macOS 事件 tap 被系统禁用无对等看门狗 | 中 | 补齐：tap 回调停止触发计时检测 → 重装 |
| R8 | 密码场景误注入 Cmd+C | 中 | foreground_context 加 is_password 检查，宁可放弃本次翻译 |
| R9 | 单人维护文档腐化 | 低 | 每条决策标注 file:line；拆分落位后立即更新引用 |

### 7.2 手测矩阵（每格记：图标弹否 / 热键-兜底关 / 热键-兜底开 / pin 行为）

| 场景 | Windows 预期 | macOS 预期 |
|---|---|---|
| 普通 WebView 应用（浏览器/Word）划词 | 图标弹；UIA 命中 | 图标弹；AX 命中 |
| 老控制台 cmd.exe | 图标不弹（NoTextPattern）；兜底开也不注入（闸） | Terminal.app 无 AX 文本 → 图标不弹 |
| Windows Terminal | 图标弹（有 TextPattern） | — |
| 管理员权限应用 | 图标不弹，日志标 elevated；热键按开关走兜底 | — |
| 全屏 DirectX 游戏 | 钩子可能收不到/图标不弹；无崩溃 | 同 |
| 密码框/系统登录 | 图标不弹、兜底不注入 | Secure Input 生效，同左 |
| 自绘应用（部分 Electron/Qt） | 拉黑 10min 内不白等，图标不弹 | 同 |
| 锁屏/UAC 安全桌面 | 无事件，恢复后正常 | 同 |
| 双显示器跨屏 + 150% 缩放 | 卡片钳入所在屏工作区 | 菜单栏/Dock 遮挡检查 |
| mac 卡片 makeKey 失灵 | — | 模式二：点外收、点内不收、pin 后永不收 |
| pin 后新划词 | 解 pin 换内容 | 同 |
| 截图会话触发时卡片 pinned | 卡片藏起，结束后恢复且仍 pinned | 同 |
| 同文本 1.5s 内重复划词 | 不重复弹/不重复翻译 | 同 |
| 热键连按 | 上一轮完成才接受新轮 | 同 |
| 慢应用（Word 大文档）热键 | 兜底开：grab 成功且 2.5s watcher 纠正背刺写入 | 同 |

### 7.3 迁移路线图（现状 → 目标，建议顺序）

1. **无行为变化**：版本号三处对齐 + CI 校验；删 diag/心跳埋点；补 ci.yml
2. **纯搬迁**：geometry / updater / screenshot / tray / hotkeys / windows 各模块拆出，main.rs 收缩为薄壳（每步独立 commit）
3. **SelectionRead 四态统一**到 platform 门面，macOS 补齐 AXError 映射；新增 foreground_context
4. **selection/ 落位**：SessionManager 收编静态变量；gesture/probe/fallback/normalize 归位；热键路径 UIA 先试 + clipboard_fallback 开关（**首个行为变化点**，对应 R2）
5. **translate trait 化**：5 源拆文件、静态注册表、pipeline 过滤
6. **前端**：useCardDismiss 抽取 + pin UI + set_card_pinned 命令 + Settings 开关
7. **手测矩阵全跑**，更新文档内行号引用
