# 划词轻提示（小点）交互流程设计

- 日期：2026-11-25
- 状态：设计已获用户确认（对话中逐项取舍），待评审后出实施计划
- 性质：工程方案，不含实现代码；现状引用带 file:line 锚点（以 2026-11-25 main `6e6d8ed` 为准）
- 前置：本设计建立在 `2026-11-06-architecture-redesign.md` 迁移已落地的模块结构上

## 1. 目标与已确认取舍

把划词从「探测成功立即弹图标」改为「静默 → 轻提示」：

1. 划词后默认什么都不弹
2. 松手后鼠标停留不动（漂移 ≤ 10px）满 350ms 且选区未被新操作改变 → 选区右下出现半透明小点
3. 鼠标靠近小点（进入周围 36px）才展开「翻译」胶囊；移开收回到小点；小点 2.5s 无交互消失
4. 点击「翻译」出卡片；快捷键直翻永远可用，不依赖小点
5. 输入框 / IDE 等可编辑焦点场景默认不出小点（UIA/AX 自动判定 + 手动排除列表）
6. 不设多档模式：现有 `icon_hotkey`（默认 Alt+W）翻转开关的机制平移为「轻提示开关」——开=自动轻提示，关=仅快捷键（不出点，热键仍可用）
7. 托盘菜单加勾选项，随时可见/切换当前状态
8. 首次弹出小点时跟随一次性引导气泡

用户逐项确认的取舍（2026-11-25 对话）：

| 取舍点 | 结论 |
|---|---|
| 旧立即弹图标 | 移除，小点取代 |
| 双击直翻 | 移除。**核实：现网代码不存在该路径**（重设计文档 §3.2 的「双击同点→CardShown」直连分支未实装，probe.rs L89-96 Up 只走 probe_selection），零改动 |
| 移开后行为 | 收回到小点，2.5s 超时消失（非立即全消失） |
| 输入场景判定 | UIA/AX 自动判定 + 手动排除列表；白名单暂不做（误伤时 Alt+W 一键关兜底，YAGNI） |
| 模式档位 | 不做四档，Alt+W 开关二值化（开=轻提示 / 关=仅快捷键） |
| Alt 修饰键交互 | 不做 |
| 350ms 判定 | 鼠标停留不动才算（漂移 ≤10px），非仅选区稳定 |
| 首次引导 | 首次划词出点时跟随小点的气泡，一次性 |
| 渲染方案 | 复用 icon 窗口、固定展开尺寸、前端切状态（方案 A；B 双窗口、C 原生绘制已否） |

## 2. 现状锚点（改动基础）

| 机制 | 位置 | 处置 |
|---|---|---|
| 探测编排：Up → probe_selection → probe_and_show → 立即弹图标 | `src-tauri/src/selection/probe.rs:100-189` | 改：成功分支进静置判定，不立即弹 |
| 鼠标监听三回调（down/up/move），move 已做飘远判定 | `src-tauri/src/selection/probe.rs:39-63` | 扩：move 回调增加静置脏标记置位与近距离检测 |
| 图标 show/hide、矩形缓存、点击命令 | `src-tauri/src/selection/icon.rs:16-82` | 改造为 dot 语义（见 §4） |
| 会话状态 SessionManager（gen/icon_text/dedup） | `src-tauri/src/selection/mod.rs:50-96` | 保留，icon_text 等改名 |
| 手势判定（位移≥6px / 连击，系统双击间隔夹取） | `src-tauri/src/selection/gesture.rs:19-62` | 保留不动（连击=双击选词手势，之后照样可出小点） |
| 开关字段 `selection_icon: bool` | `src-tauri/src/translate/mod.rs:46-47,86-87,121` | 改名 `selection_dot`（serde alias 兼容） |
| Alt+W 翻转开关 `toggle_selection_icon` | `src-tauri/src/translate/mod.rs:361-391` | 改名 `toggle_selection_dot`，文案改「划词轻提示」 |
| 热键注册 icon_hotkey 分支 | `src-tauri/src/hotkeys.rs:107-110` | 只改调用名 |
| 托盘菜单（仅「退出」） | `src-tauri/src/tray.rs:14-16` | 加 checkable「划词轻提示」项 |
| 前端 SelectionIcon（logo 按钮） | `src/components/SelectionIcon/index.tsx:1-23` | 重写为三态渲染 |
| 防弹回静默窗 ICON_REPROBE_QUIET_MS=400ms | `src-tauri/src/selection/probe.rs:27` | 保留 |
| 自家窗口排除 / hopeless 拉黑 / 剪贴板铁律 | `probe.rs:129-137`、`platform/mod.rs`（TTL 10min） | 保留不动 |

## 3. 状态机

```
Idle   --手势松开(记 t_up, x_up, y_up)-->        Probing      # 现状不变
Probing --Text 且 gen 未过期且未飘远-->           Settling     # 改：不再立即弹
Probing --NoTextPattern/NoSelection/飘远/自家窗口/可编辑焦点(focused_editable)/排除列表命中--> Idle   # 含 hopeless 拉黑
Settling --t_up+350ms 时: 漂移≤10px 且 gen 未变 且 开关开--> DotShown
Settling --期间鼠标挪远(>10px)/新按下(gen 变)/开关被关-->   Idle + hide
DotShown --光标进入点矩形+36px（far→near 边沿）--> Expanded
Expanded --光标离开 36px-->                      DotShown     # 超时计时重启
DotShown --2.5s 无交互-->                        Idle + hide
Expanded/DotShown --点击「翻译」-->               CardShown    # show_card 合流点，永不碰剪贴板
任意态   --鼠标按下/热键/兜底 grab-->              按现有路径   # Down 仍 bump_gen 收点
```

实现要点（单一事实源仍为 SessionManager + 静态原子，回调轻量铁律不变）：

- **静置判定 = 单次定时器，非轮询循环**。probe 成功分支 spawn 定时器睡到 `t_up + 350ms`（探测本身 120~360ms，多数情况到点即弹，无感延迟）；到点复查三个廉价条件：漂移脏标记未置位、`gen` 未变、开关仍开。
- **漂移脏标记**：move 回调内只做一次坐标比较（`|x-x_up|+|y-y_up| > 10` 则置静态原子 `SETTLE_DIRTY`），符合钩子回调只做内存读铁律。Probing 进入 Settling 时清零。
- **proximity 边沿触发**：move 回调对 `DOT_RECT`（icon.rs:14 同款缓存）做 36px 判定，与现 ICON_LEAVE_PX（probe.rs:23）同构；worker 收 `Near`/`Far` 信号，仅 far→near 边沿切展开态，杜绝「出现瞬间鼠标恰好停旁边立即展开」。
- **超时**：DotShown 态 worker 记 `dot_shown_at`；展开态无超时（鼠标停着看翻译是正当停留）；收回 DotShown 时重启计时。超时检查挂在现有信号驱动上（每次 Near/Far/Leave 信号顺带检查），不新增轮询线程。
- **点「翻译」**：`translate_selection_icon`（icon.rs:73-82）原样保留，`icon_clicked_recently` 防弹回机制不变。

## 4. 模块改动

### 4.1 selection/（Rust 核心）

- `icon.rs` → 语义更名 dot（文件名保留或更名均可，实施计划定）：
  - `show_selection_icon` → `show_selection_dot`：位置改锚定**选区右下**（现状锚松手点 +8,+12，icon.rs:33；右下 ≈ 选区矩形端点，UIA `BoundingRectangle` 已有则优先，取不到退回松手点），`ICON_RECT`/`DOWN_ON_ICON`/`cursor_on_icon_rect` 改名 DOT_*
  - 新增 emit `dot-state` 事件（`near`/`far`）驱动前端切换
- `probe.rs`：
  - `probe_and_show` 成功分支（L171-184）改为 `start_settle(worker, text, t_up, ux, uy, gen)`
  - move 回调（L52-62）扩展：静置脏标记 + DOT proximity 判定（仍是纯内存读）
  - 新增 `SETTLE_DRIFT_PX=10`、`SETTLE_MS=350`、`DOT_PROXIMITY_PX=36`、`DOT_TIMEOUT_MS=2500` 常量
  - 开关读点从 `selection_icon`（L109-112）改 `selection_dot`，**开关关时不探测**（现状行为保留：省 UIA 开销）
- `Session`（mod.rs:50-64）：`icon_text/icon_clicked_at` 改名 `dot_text/dot_clicked_at`；新增 `dot_shown_at: u64`

### 4.2 platform 门面：可编辑焦点检测

新增 `focused_editable() -> bool`（探测成功后、进 Settling 前调用；true 则不出点，仅诊断日志）：

- Windows：UIA 前台焦点元素查 `IsPasswordProperty` / `ValuePattern.IsReadOnly`（复用现有 UIA 会话，windows.rs L453-501 同一基础设施）
- macOS：AX 焦点元素 `kAXEditableAttribute`；Secure Input 既有覆盖保留（platform/mod.rs 降级链表）
- hopeless 拉黑与 NoTextPattern 路径不变——终端多数在这关就出不来

### 4.3 配置（translate/mod.rs）

```rust
#[serde(default = "default_selection_dot", alias = "selection_icon")]
pub selection_dot: bool,          // 平移自 selection_icon（translate/mod.rs:46-47），默认值不变

#[serde(default)]
pub dot_exclude_apps: Vec<String>, // 进程名（小写、不带 .exe），命中则不出小点

#[serde(default)]
pub dot_guide_shown: bool,        // 首次引导已展示
```

- `toggle_selection_icon` → `toggle_selection_dot`（mod.rs:361-391）：翻转 `selection_dot`，HUD 文案改「划词轻提示已开启/关闭」；关闭时收点逻辑同现状 L380-385
- 进程名获取：`foreground_context` 的 pid → 进程名链路（重设计文档 §2.1 已规划，未实装则本次一并补最小实现——只取进程名，不查 elevation/密码框）

### 4.4 托盘（tray.rs）

菜单从「退出」扩为：

```
划词轻提示  ✓/✗   (checkable, id="toggle-dot")
─────────
退出
```

- `on_menu_event` 处理 `toggle-dot` → 调 `toggle_selection_dot`，随后重建菜单刷新勾选态（或用 CheckMenuItem 自动态，实施时按 tauri menu API 定）
- tooltip 保持现状

### 4.5 前端

- `SelectionIcon/index.tsx` 重写三态组件（复用 icon 窗口，窗口尺寸固定为展开态胶囊尺寸 + 透明余量，**不 resize 窗口**）：
  - `idle`：居中 8px 半透明圆点
  - `expanded`：「翻译」文字胶囊（点击仍 `invoke("translate_selection_icon")`，index.tsx:16 逻辑不变）
  - `guide`： expanded 态旁挂一次性气泡「靠近小点翻译，或按快捷键」，点击任意处或 5s 后消失并 `invoke("save_dot_guide_shown")`
  - 监听 `dot-state` 事件 + 启动时读初始配置
- `tauri.conf.json` icon 窗口尺寸改展开态尺寸（宽 ~64 × 高 ~28 逻辑 px + 余量），`transparent` 保持
- 设置页：原「划词图标」开关文案改「划词轻提示」；TranslateTab（或现有开关所在 tab）加排除列表编辑（进程名，逗号/换行分隔）
- `src/types.ts:22` `selection_icon?: boolean` → `selection_dot?: boolean`

## 5. 首次引导生命周期

1. 出点路径检查 `dot_guide_shown == false` → emit 的初始态事件带 `guide: true`
2. 前端渲染引导气泡（随小点出现，独立于 expanded 态）
3. 气泡消失（点击/5s）→ 前端 `save_dot_guide_shown` 落盘
4. 落盘失败不重试（下次再显示一次，无害）

## 6. 降级与边界

| 场景 | 行为 |
|---|---|
| 输入框/密码框（UIA IsPassword / ValuePattern 可编辑） | 不出点，仅诊断日志 |
| IDE/终端（多数 NoTextPattern） | 探测这关就不出（含 hopeless 拉黑 10min） |
| 排除列表命中进程 | 不出点 |
| 鼠标钩子装不上 | 点不可用、热键仍可用（现状 probe.rs:72-75） |
| UIA 误判可编辑（误伤） | 用户 Alt+W 关闭轻提示，或排除列表反向不适用——白名单暂不做（已确认取舍） |
| 点完「翻译」400ms 内 | 防弹回静默窗（现状保留） |
| 双击选词 | 连击手势 → 探测 → 照常可出小点（双击直翻本就不存在，无需移除） |
| 开关关（仅快捷键） | move 回调 proximity 判定短路、探测直接放弃（L116 enabled 检查），CPU 开销≈现状关图标 |

## 7. 测试

- **Rust 纯函数单测**（对齐 gesture.rs 测试风格）：
  - 静置判定：漂移 10px 阈值边界、gen 变化、开关中途关闭
  - proximity 边沿：far→near 才展开、near→far 收回、初始 near 不展开
  - 超时计算：2.5s 边界、展开期不超时、收回重启计时
- **前端 node test**：三态切换谓词、引导气泡一次性逻辑
- **手测矩阵**（在重设计文档 §7.2 基础上替换图标相关行）：

| 场景 | 预期 |
|---|---|
| 浏览器划词，鼠标停住 | 350ms 后右下出点；靠近展开；移开收回；2.5s 消失 |
| 划完立刻挪鼠标去别处 | 不出点 |
| 输入框（浏览器地址栏/记事本）划词重选 | 不出点 |
| VSCode / 终端 | 不出点（探测或可编辑判定拦截） |
| 密码框 | 不出点 |
| Alt+W 连按 | HUD 反馈 + 托盘勾选态同步；关时收点 |
| 点「翻译」后立即再划词 | 400ms 静默窗内不弹回 |
| 双显示器 + 150% 缩放 | 点/展开态钳入所在屏工作区 |
| 引导气泡 | 仅首次出现，消失后落盘永不再弹 |
| 快捷键直翻（轻提示关） | 完全不受影响 |

## 8. 风险

| # | 风险 | 等级 | 缓解 |
|---|---|---|---|
| R1 | 静置定时器 × 异步探测 × gen 三者时序竞态 | 高 | 单次定时器 + 到点复查三廉价条件；全部判定有纯函数单测；脏标记只置不清（每次 Settling 进入时重置） |
| R2 | 选区右下锚点：UIA BoundingRectangle 并非总可得（多选区/AX 缺属性） | 中 | 取不到退回松手点 +8,+12（现状位置），体验降级可接受 |
| R3 | 36px proximity 误展开（选区短、点紧贴正文，鼠标扫过正文即触发） | 中 | 边沿触发 + 阈值单测 + 手测矩阵调参；常量集中定义便于调 |
| R4 | icon 窗口固定为胶囊尺寸后，透明余量区域拦截鼠标点击 | 中 | 现状图标窗口已是透明窗口可点（DOWN_ON_ICON 机制），透明区命中即「点在点上」，行为一致；验证点外穿透是否需要 `set_ignore_cursor_events`（实施时手测定） |
| R5 | Alt+W 语义变化（图标开关→轻提示开关）对老用户的差异 | 低 | 本质同一开关改名，HUD/托盘/引导气泡三重告知 |
| R6 | 托盘菜单刷新勾选态的 tauri API 版本差异 | 低 | 实施时确认 CheckMenuItem set_checked 行为，失败则重建菜单 |
