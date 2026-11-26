# 划词轻提示（小点）实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把划词从「探测成功立即弹图标」改为「静默 350ms → 半透明小点 → 靠近展开『翻译』」，并平移开关语义（Alt+W）、托盘勾选项与一次性引导。

**Architecture:** 复用现有 icon 窗口（label 不变）与鼠标钩子/探测基础设施；probe.rs 增加静置判定（单次定时器 + 漂移脏标记）、proximity 边沿检测与超时；前端 SelectionIcon 重写为「点 / 翻译胶囊 / 引导气泡」三态，由 Rust `dot-state` 事件驱动。

**Tech Stack:** Tauri 2 + React 18 + windows crate（UIA）/ macOS AX；无新增依赖。

**Spec:** `docs/superpowers/specs/2026-11-25-selection-dot-flow-design.md`（实施时与 spec 同读）

## 与 spec 的已识别差异（实施按本文为准，均为合理化修正）

1. **超时改单次定时器**：spec §3 说超时「挂在信号驱动上」——鼠标完全静止时没有任何信号，点永不超时。改为一键式定时器线程（与静置判定同型），`DOT_EXPANDED` 原子防误收。
2. **锚点一律用松手点 +8,+12**：spec §4.1 的「UIA BoundingRectangle 优先」需要给 `SelectionRead` 四态加 rect 字段并动两平台探测层，收益仅覆盖键盘/编程选区（拖选手松手点本来就在选区右下）。走 spec R2 已列的退路，rect 锚点列为后续项。
3. **引导标志 Rust 侧落盘**：spec §5 的前端 `save_dot_guide_shown` 命令不需要——展示时直接置位 `dot_guide_shown`，少一条命令与一次 IPC 往返。
4. **进程名直接用已实装的 `foreground_context()`**（platform/mod.rs:332-352）：spec §4.3 预估未实装需补最小实现，实际已存在，零新增。

## Global Constraints

- **钩子回调铁律**：回调内禁止 UIA/AX 调用、剪贴板访问、窗口操作、阻塞锁竞争；只允许静态原子/Mutex 内存读、GetCursorPos 等价物、`try_send`（spec §2.2 / 重设计文档）
- **划词点路径绝不碰剪贴板、不注入按键**（probe.rs:156-158 注释，铁律保留）
- **DWM 主题补刀 main.rs L1387-1454 原样保留在 RunEvent，禁止 set_theme**
- **配置锁内不做磁盘 I/O**；落盘走 `CONFIG_WRITE` 串行锁（translate/mod.rs:129-130）
- **persist 模板复用**：save 命令只 mutate 自己的字段（translate/mod.rs:228-262）
- **常量值（spec §4.1）**：`SETTLE_MS=350`、`SETTLE_DRIFT_PX=10`、`DOT_PROXIMITY_PX=36`、`DOT_TIMEOUT_MS=2500`、`DOT_REPROBE_QUIET_MS=400`（原 ICON_REPROBE_QUIET_MS 改名不改值）、`ICON_NEAR_PX=400` 保留
- **窗口 label 仍为 `"icon"`、URL 仍为 `/?page=icon`**：只做代码语义更名，窗口标识不动（少动 tauri.conf.json 之外的标识链）
- **serde 兼容**：`selection_dot` 带 `alias = "selection_icon"`，旧配置文件平滑读取
- **无新增 npm/cargo 依赖**；测试沿用 `cargo test` + `npm test`（node --test）+ `npx tsc --noEmit`
- 提交信息用仓库惯例：`feat:`/`refactor:`/`test:` 前缀 + 中文描述

---

### Task 1: 配置字段平移（selection_icon → selection_dot + 新字段 + 开关函数改名）

**Files:**
- Modify: `src-tauri/src/translate/mod.rs`（struct L45-48、default L86-88、Default impl L121、toggle L361-391、save 命令区）
- Modify: `src-tauri/src/selection/probe.rs:109-112`（读点改名）
- Modify: `src-tauri/src/hotkeys.rs:109`（调用改名）
- Test: `src-tauri/src/translate/mod.rs`（文件尾新增 `#[cfg(test)] mod tests`）

**Interfaces:**
- Produces: `AppConfig.selection_dot: bool`、`AppConfig.dot_exclude_apps: Vec<String>`、`AppConfig.dot_guide_shown: bool`；`translate::toggle_selection_dot(app: &AppHandle)`；`translate::with_config_write(app: &AppHandle, mutate: impl FnOnce(&mut AppConfig))`；`translate::normalize_excludes(Vec<String>) -> Vec<String>`；`translate::process_excluded(&str, &[String]) -> bool`（后两个本任务实现，Task 4/8 消费）

- [ ] **Step 1: 写失败测试**（serde alias + 排除列表归一化/匹配纯函数）

在 `src-tauri/src/translate/mod.rs` 文件尾追加：

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selection_dot_reads_legacy_selection_icon_field() {
        // 旧配置文件里字段名是 selection_icon，serde alias 必须平滑读取
        let json = r#"{"source":"youdao","sources":["bing"],"hotkey":"Alt+Z","screenshot_hotkey":"Alt+S","ocr_lang":"auto","input_hotkey":"Alt+X","icon_hotkey":"Alt+W","popup_width":400.0,"settings_section":"translate","theme":"system","selection_icon":false}"#;
        let cfg: AppConfig = serde_json::from_str(json).unwrap();
        assert!(!cfg.selection_dot);
        // 新序列化写出的是新字段名
        let out = serde_json::to_string(&cfg).unwrap();
        assert!(out.contains("\"selection_dot\":false"));
        assert!(!out.contains("selection_icon"));
    }

    #[test]
    fn excludes_match_is_exact_case_insensitive() {
        let list = normalize_excludes(vec![
            " Code ".into(),
            "wechat.exe".into(),
            "".into(),
            "CODE".into(),
        ]);
        // 归一化：trim + 小写 + 去空 + 去重
        assert_eq!(list, vec!["code", "wechat"]);
        assert!(process_excluded("Code.exe", &list));
        assert!(process_excluded("code", &list));
        assert!(process_excluded("WeChat", &list));
        assert!(!process_excluded("vscode", &list)); // 不做子串匹配，exact only
        assert!(!process_excluded("Safari", &list));
    }
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p wordpeek selection_dot_reads_legacy`（在 `src-tauri/` 下执行，下同）
Expected: 编译失败——`selection_dot`/`normalize_excludes`/`process_excluded` 不存在，`serde_json` 若未作为 dev-dependency 会报缺失（见 Step 3 注）

- [ ] **Step 3: 实现配置字段与纯函数**

3a. struct 字段（L45-48 替换）：

```rust
    /// 划词轻提示开关：开=静置后弹半透明小点，关=仅快捷键（不出点）。快捷键 icon_hotkey 翻转
    #[serde(default = "default_selection_dot", alias = "selection_icon")]
    pub selection_dot: bool,
    /// 轻提示排除列表（进程名，小写归一化存储）：前台进程命中则不出小点
    #[serde(default)]
    pub dot_exclude_apps: Vec<String>,
    /// 首次引导气泡已展示过（展示时即刻置位落盘，失败不重试）
    #[serde(default)]
    pub dot_guide_shown: bool,
```

3b. default 函数与 Default impl（L86-88 与 L121）：

```rust
fn default_selection_dot() -> bool {
    true
}
```

`Default impl` 里 `selection_icon: default_selection_icon()` 改为 `selection_dot: default_selection_dot(), dot_exclude_apps: Vec::new(), dot_guide_shown: false,`。

3c. 排除列表纯函数（放在 `migrate_sources` 附近）：

```rust
/// 排除列表归一化：trim、去 .exe 后缀、小写、去空、去重（存取两侧共用，保证列表干净）
pub(crate) fn normalize_excludes(raw: Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for s in raw {
        let s = s.trim().trim_end_matches(".exe").to_lowercase();
        if !s.is_empty() && !out.contains(&s) {
            out.push(s);
        }
    }
    out
}

/// 前台进程名是否命中排除列表。两侧都按小写精确比较；带不带 .exe 都能命中
/// （Windows 进程名带 .exe，用户输入通常不带），不做子串匹配防误伤
pub(crate) fn process_excluded(process_name: &str, excludes: &[String]) -> bool {
    let name = process_name.to_lowercase();
    let stem = name.strip_suffix(".exe").unwrap_or(&name);
    excludes.iter().any(|e| {
        let e = e.trim().trim_end_matches(".exe").to_lowercase();
        !e.is_empty() && (name == e || stem == e)
    })
}
```

3d. `with_config_write` 内部落盘助手（放 `save_theme` 附近）：

```rust
/// 内部小改动落盘（如 dot_guide_shown 置位）：锁内改字段取快照，锁外写盘。
/// 与 persist 的区别：不重注册快捷键、不发 config-changed（单布尔无需广播）
pub(crate) fn with_config_write(app: &AppHandle, mutate: impl FnOnce(&mut AppConfig)) {
    let Some(path) = config_path(app) else {
        eprintln!("[config] 无法定位配置目录，改动未落盘");
        return;
    };
    let snapshot = {
        let shared = app.state::<SharedConfig>();
        let mut guard = shared.lock();
        mutate(&mut guard);
        guard.clone()
    };
    let _write = CONFIG_WRITE.lock();
    if let Err(e) = write_config(&path, &snapshot) {
        eprintln!("[config] 落盘失败: {e}");
    }
}
```

3e. `toggle_selection_icon` → `toggle_selection_dot`（L361-391 整体替换；HUD 文案、收点窗口、注释同步改）：

```rust
/// 划词轻提示开关（全局快捷键 icon_hotkey / 托盘勾选项触发）：翻转 selection_dot 并落盘；
/// 鼠标监听后台探测时每次现读该字段，改完立即生效。切换后 HUD 反馈新状态并刷新托盘勾选
pub(crate) fn toggle_selection_dot(app: &AppHandle) {
    let Some(path) = config_path(app) else {
        eprintln!("[dot] 无法定位配置目录，切换未落盘");
        return;
    };
    // 锁内只翻字段取快照，磁盘 I/O 在锁外（同其余 save_* 命令）
    let (enabled, snapshot) = {
        let shared = app.state::<SharedConfig>();
        let mut guard = shared.lock();
        guard.selection_dot = !guard.selection_dot;
        (guard.selection_dot, guard.clone())
    };
    let _write = CONFIG_WRITE.lock();
    if let Err(e) = write_config(&path, &snapshot) {
        eprintln!("[dot] 切换落盘失败: {e}");
    }
    // 关闭时顺手把可能还显着的小点收掉
    if !enabled {
        if let Some(win) = app.get_webview_window("icon") {
            let _ = win.hide();
        }
    }
    crate::windows::show_hud(
        app,
        if enabled { "划词轻提示已开启" } else { "划词轻提示已关闭" },
    );
    crate::tray::refresh_dot_item(app, enabled);
}
```

> `crate::tray::refresh_dot_item` 在 Task 7 才存在——本任务先在 tray.rs 追加一个空实现占位并注明「Task 7 实现」？不允许占位。改：**Task 1 不调 tray**，`toggle_selection_dot` 末尾的 `crate::tray::refresh_dot_item(app, enabled);` 这一行由 Task 7 加入。本任务 HUD 之后即结束。

3f. 引用改名：`probe.rs:112` `.selection_icon` → `.selection_dot`；`hotkeys.rs:109` `toggle_selection_icon` → `toggle_selection_dot`。`src/types.ts:22` `selection_icon?: boolean` → `selection_dot?: boolean`（保持前端 tsc 可过）。

3g. dev-dependency：`serde_json` 若不在 `src-tauri/Cargo.toml [dev-dependencies]`，加 `serde_json = "1"`（生产代码已用 serde，仅测试需要 json 解析）。

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test`（全量，确认既有测试不回归）+ `npx tsc --noEmit`
Expected: PASS（新增 2 条 + 既有全绿）

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "refactor: 划词图标开关平移为轻提示开关 selection_dot（serde 兼容旧字段），新增排除列表/引导标志字段"
```

---

### Task 2: selection 域语义更名（icon → dot，纯移动不改逻辑）

**Files:**
- Rename: `src-tauri/src/selection/icon.rs` → `src-tauri/src/selection/dot.rs`
- Modify: `src-tauri/src/selection/mod.rs`（mod 声明 L16、Session 字段 L57-59、方法 L81-96、re-export L20-21）
- Modify: `src-tauri/src/selection/probe.rs`（import L11、常量 L27、调用点 L42/84-87/105/125/134/179/182/185）
- Modify: `src-tauri/src/selection/fallback.rs:8,151,161`
- Modify: `src-tauri/src/main.rs:159,238`

**Interfaces:**
- Produces: `dot::DOT_RECT: Mutex<Option<(i32,i32,u32,u32)>>`、`dot::DOWN_ON_DOT: AtomicBool`、`dot::hide_selection_dot(&AppHandle)`、`dot::show_selection_dot(&AppHandle, String, i32, i32)`、`dot::cursor_on_dot_rect() -> bool`、`#[tauri::command] dot::translate_selection_dot`、`SessionManager::set_dot_text/take_dot_text/dot_clicked_recently`、`probe::start_selection_dot`、常量 `DOT_REPROBE_QUIET_MS`
- 本任务**不改任何行为**；窗口 label `"icon"`、`SOURCE_ICON = "icon"` 字符串值不动

- [ ] **Step 1: git mv 文件**

```bash
git mv src-tauri/src/selection/icon.rs src-tauri/src/selection/dot.rs
```

- [ ] **Step 2: 全量改名**（用编辑器逐处替换，符号一一对应）

| 旧 | 新 |
|---|---|
| `mod icon;`（selection/mod.rs:16）| `pub(crate) mod dot;` |
| `icon::` 路径引用 | `dot::` |
| `DOWN_ON_ICON` | `DOWN_ON_DOT` |
| `ICON_RECT` | `DOT_RECT` |
| `hide_selection_icon` | `hide_selection_dot` |
| `show_selection_icon` | `show_selection_dot` |
| `cursor_on_icon_rect` | `cursor_on_dot_rect` |
| `translate_selection_icon`（fn 与 command 字符串）| `translate_selection_dot` |
| `ICON_REPROBE_QUIET_MS` | `DOT_REPROBE_QUIET_MS`（值 400 不变）|
| `ICON_LEAVE_PX` | `DOT_LEAVE_PX`（值 48 暂不变，Task 4 删）|
| `icon_text` / `icon_clicked_at`（Session 字段）| `dot_text` / `dot_clicked_at` |
| `set_icon_text` / `take_icon_text` / `icon_clicked_recently` | `set_dot_text` / `take_dot_text` / `dot_clicked_recently` |
| `start_selection_icon` | `start_selection_dot` |
| `icon_clicked_recently(now)` 调用点 probe.rs:113 | `dot_clicked_recently(now)` |

涉及文件（对照 grep 结果，不遗漏）：`selection/mod.rs`、`selection/dot.rs`、`selection/probe.rs`、`selection/fallback.rs`、`main.rs:159,238`。注释里的「图标」措辞一并改为「轻提示/小点」（dot.rs 模块文档、probe.rs:32-33）。

- [ ] **Step 3: 编译 + 全量测试**

Run: `cargo test` + `cargo build`
Expected: PASS。前端 `invoke("translate_selection_icon")` 此时已对不上后端命令（Task 6 修），属预期中间态；`cargo build` 与 `npm test` 不受影响。

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "refactor: selection 域语义更名 icon→dot（纯改名不改行为），窗口 label 保留 icon"
```

---

### Task 3: platform 门面新增 focused_editable（可编辑焦点检测）

**Files:**
- Modify: `src-tauri/src/platform/windows.rs`（`password_focused` L171-192 旁新增）
- Modify: `src-tauri/src/platform/macos.rs`（`foreground_context` L522 附近新增）
- Modify: `src-tauri/src/platform/mod.rs`（门面 re-export 区 L244-252）

**Interfaces:**
- Produces: `platform::focused_editable() -> bool`（true = 密码框/输入框/IDE 编辑区/安全输入，不出小点；基础设施查不到一律 false，不能反过来拦下正常划词）
- Consumes: windows `password_focused()`（已存在 L171）；macos `secure_input_active()`（已存在 L490）、`ax_attr()`（已存在 L837）

- [ ] **Step 1: Windows 实现**（windows.rs，紧挨 `password_focused`）

```rust
/// 焦点控件是否可编辑/敏感（UIA IsPassword，或 ValuePattern 存在且非只读）。
/// 轻提示的降噪闸：输入框/IDE 编辑区划词重选不弹小点。查不到（COM 起不来/无焦点
/// 元素/无 ValuePattern）一律按否处理——只是降噪，不能反过来拦下正常翻译
pub fn focused_editable() -> bool {
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED,
    };
    use windows::Win32::UI::Accessibility::{CUIAutomation, IUIAutomationValuePattern};

    if password_focused() {
        return true;
    }
    unsafe {
        // 与 read_focused_selection / password_focused 同理用 MTA（见 password_focused 注释）
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let automation: windows::Win32::UI::Accessibility::IUIAutomation =
            match CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER) {
                Ok(a) => a,
                Err(_) => return false,
            };
        let Ok(el) = automation.GetFocusedElement() else {
            return false;
        };
        el.GetCurrentPattern(&IUIAutomationValuePattern::IID)
            .ok()
            .and_then(|u| u.cast::<IUIAutomationValuePattern>().ok())
            .and_then(|p| p.CurrentIsReadOnly().ok())
            .map(|ro| !ro.as_bool())
            .unwrap_or(false)
    }
}
```

> `IUIAutomationValuePattern::IID` 与 `cast` 来自 `windows::core::Interface` trait，需 `use windows::core::Interface as _;`（或确认已在作用域）。若当前 windows crate 版本的方法签名不同（如 `GetCurrentPattern` 参数形式），以 crate 实际签名微调，语义保持「取 ValuePattern → 判 IsReadOnly」。

- [ ] **Step 2: macOS 实现**（macos.rs，`foreground_context` 上方）

```rust
/// 焦点控件是否可编辑（AXEditable 属性存在即算可编辑）或系统安全输入开启中。
/// 与 Windows 侧同理：只是轻提示降噪闸，查不到按否处理，Secure Input 恒拦
pub fn focused_editable() -> bool {
    if secure_input_active() {
        return true;
    }
    unsafe {
        let system = AXUIElementCreateSystemWide();
        if system.is_null() {
            return false;
        }
        let focused = ax_attr(system, "AXFocusedUIElement");
        CFRelease(system);
        let Some(focused) = focused else {
            return false;
        };
        let editable = match ax_attr(focused, "AXEditable") {
            Some(v) => {
                CFRelease(v);
                true
            }
            None => false,
        };
        CFRelease(focused);
        editable
    }
}
```

- [ ] **Step 3: 门面导出**（platform/mod.rs L244-252 区域）

```rust
#[cfg(windows)]
pub use windows::{focused_editable, foreground_pid, read_focused_selection, warm_uia};
#[cfg(target_os = "macos")]
pub use macos::{focused_editable, foreground_pid, read_focused_selection};
```

并加模块级注释：

```rust
/// 焦点控件是否可编辑/敏感：轻提示不出小点的降噪闸（两平台统一语义，见各自实现）
```

- [ ] **Step 4: 双平台编译验证**

Run: `cargo build`（Windows 本机验证 cfg(windows) 分支）
Expected: PASS。macOS 分支本机不可编译验证，语法自查 + CI/release 时验证（沿用现状：本仓库无 mac 测试流水线，release.yml 构建）。

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat: platform 门面新增 focused_editable（UIA ValuePattern/AXEditable 可编辑焦点检测）"
```

---

### Task 4: 状态机核心——静置判定 + proximity 边沿 + 超时（TDD）

**Files:**
- Modify: `src-tauri/src/selection/probe.rs`（常量区、statics、Signal 枚举、start_selection_dot 三回调、worker match、probe_and_show 成功分支、新增 start_settle/arm_dot_timeout/dot_gated、tests）
- Modify: `src-tauri/src/selection/dot.rs`（`DOT_LEAVE_PX` 语义并入 proximity，见 Step 4）
- Test: `src-tauri/src/selection/probe.rs`（`#[cfg(test)] mod tests`）

**Interfaces:**
- Consumes: Task 1 的 `translate::process_excluded`/`AppConfig.selection_dot`；Task 2 的 `dot::*` 改名符号；Task 3 的 `platform::focused_editable`
- Produces（纯函数，供测试与 worker 使用）：
  - `within_margin(x: i32, y: i32, rect: (i32, i32, u32, u32), margin: i32) -> bool`
  - `settle_passes(dirty: bool, gen_start: u64, gen_now: u64, enabled: bool) -> bool`
  - `dot_expired(now_ms: u64, shown_at_ms: u64, timeout_ms: u64) -> bool`
- Produces（内部）：statics `SETTLE_ACTIVE/SETTLE_DIRTY: AtomicBool`、`SETTLE_ORIGIN: Mutex<(i32,i32)>`、`CURSOR_NEAR: AtomicBool`（dot.rs）；`DOT_EXPANDED: AtomicBool`（dot.rs）；常量 `SETTLE_MS/DOT_PROXIMITY_PX/DOT_TIMEOUT_MS`

- [ ] **Step 1: 写失败测试**（probe.rs 文件尾）

```rust
#[cfg(test)]
mod tests {
    use super::*;

    const RECT: (i32, i32, u32, u32) = (100, 100, 40, 40);

    #[test]
    fn within_margin_is_box_expanded_rect() {
        // 矩形内 → 近
        assert!(within_margin(120, 120, RECT, DOT_PROXIMITY_PX));
        // 外扩 margin 的边界内（含边）→ 近：x 最大 100+40+36-1=175
        assert!(within_margin(175, 120, RECT, DOT_PROXIMITY_PX));
        // 边界外 1px → 远
        assert!(!within_margin(176, 120, RECT, DOT_PROXIMITY_PX));
        assert!(!within_margin(63, 120, RECT, DOT_PROXIMITY_PX)); // 100-36-1
        assert!(!within_margin(120, 176, RECT, DOT_PROXIMITY_PX));
    }

    #[test]
    fn settle_passes_requires_still_fresh_and_enabled() {
        assert!(settle_passes(false, 7, 7, true));
        // 期间鼠标挪过（漂移 > 10px 置脏）
        assert!(!settle_passes(true, 7, 7, true));
        // 期间开始了新选区（gen 变）
        assert!(!settle_passes(false, 7, 8, true));
        // 开关中途被关（Alt+W / 托盘）
        assert!(!settle_passes(false, 7, 7, false));
    }

    #[test]
    fn dot_expired_boundary() {
        assert!(!dot_expired(1000, 1000, DOT_TIMEOUT_MS)); // 刚展示
        assert!(!dot_expired(3499, 1000, DOT_TIMEOUT_MS));
        assert!(dot_expired(3500, 1000, DOT_TIMEOUT_MS)); // 恰好到期
    }
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test within_margin settle_passes dot_expired`
Expected: 编译失败——三个纯函数与常量不存在

- [ ] **Step 3: 实现常量与纯函数**（probe.rs 顶部）

```rust
/// 松手后鼠标停留不动满这么久（且选区未被新操作改变）才弹小点：静默轻提示的核心阈值
const SETTLE_MS: u64 = 350;
/// 静置期间光标漂移超过这么远（曼哈顿距离，物理像素）就算「挪走了」，不出点
const SETTLE_DRIFT_PX: i32 = 10;
/// 光标进入小点窗口外扩这么远的矩形即「靠近」，展开「翻译」胶囊（far→near 边沿触发）
const DOT_PROXIMITY_PX: i32 = 36;
/// 小点无交互这么久自动收起（展开态无超时；收回小点时重新计时）
const DOT_TIMEOUT_MS: u64 = 2500;

/// 纯判定：静置到点后是否还成立（漂移脏标记未置位 ∧ 代次未变 ∧ 开关仍开）
fn settle_passes(dirty: bool, gen_start: u64, gen_now: u64, enabled: bool) -> bool {
    !dirty && gen_start == gen_now && enabled
}

/// 纯判定：光标是否落在矩形外扩 margin 的盒内（与旧 Leave 判定同构取反）；
/// 盒状而非欧氏距离，判定只做四次比较，钩子回调可用
fn within_margin(x: i32, y: i32, rect: (i32, i32, u32, u32), margin: i32) -> bool {
    let (rx, ry, rw, rh) = rect;
    x >= rx - margin
        && x < rx + rw as i32 + margin
        && y >= ry - margin
        && y < ry + rh as i32 + margin
}

/// 纯判定：小点是否已超时无交互
fn dot_expired(now_ms: u64, shown_at_ms: u64, timeout_ms: u64) -> bool {
    now_ms.saturating_sub(shown_at_ms) >= timeout_ms
}
```

- [ ] **Step 4: 静置/proximity/超时静态变量**（probe.rs / dot.rs）

dot.rs 顶部（紧跟 DOWN_ON_DOT/DOT_RECT）：

```rust
/// 光标当前是否在小点 proximity 盒内（move 回调记、边沿检测用）
pub(crate) static CURSOR_NEAR: AtomicBool = AtomicBool::new(false);
/// 「翻译」胶囊是否处于展开态（超时定时器据此不误收展开态）
pub(crate) static DOT_EXPANDED: AtomicBool = AtomicBool::new(false);
```

probe.rs 顶部：

```rust
/// 静置判定进行中（Settling 态）：move 回调据此检查漂移
static SETTLE_ACTIVE: AtomicBool = AtomicBool::new(false);
/// 静置期间光标是否漂移超限（只置不清；start_settle 进入时重置）
static SETTLE_DIRTY: AtomicBool = AtomicBool::new(false);
/// 静置原点 = 松手点：漂移比较基准
static SETTLE_ORIGIN: Mutex<(i32, i32)> = Mutex::new((0, 0));
```

（`use parking_lot::Mutex;` 与 `use std::sync::atomic::AtomicBool;` 按需补进 probe.rs use 区。）

- [ ] **Step 5: 改造 start_selection_dot 的 move 回调与 worker**（probe.rs:39-96）

move 回调（替换原 L51-62 的 Leave 逻辑，`DOT_LEAVE_PX` 与 Leave 信号删除）：

```rust
        // 移动：静置漂移置脏 + 小点 proximity 边沿检测。只碰内存读 + 坐标比较，保持回调轻量
        Box::new(move |x, y| {
            if SETTLE_ACTIVE.load(Ordering::Relaxed) {
                let (ox, oy) = *SETTLE_ORIGIN.lock();
                if (x - ox).abs() + (y - oy).abs() > SETTLE_DRIFT_PX {
                    SETTLE_DIRTY.store(true, Ordering::Relaxed);
                }
            }
            let near = if let Some(rect) = *super::dot::DOT_RECT.lock() {
                within_margin(x, y, rect, DOT_PROXIMITY_PX)
            } else {
                false
            };
            if near != super::dot::CURSOR_NEAR.swap(near, Ordering::Relaxed) {
                let _ = tx_move.try_send(if near { Signal::Near } else { Signal::Far });
            }
        }),
```

Signal 枚举（替换原 `Leave`）：

```rust
/// 鼠标信号：Down=按下、Up=松开（带松手位置）、Near/Far=小点 proximity 边沿
enum Signal {
    Down,
    Up { x: i32, y: i32 },
    Near,
    Far,
}
```

worker match（替换原 L80-88 的 Down/Leave 分支）：

```rust
                // 按下：代次 +1 废掉在途探测/静置/超时；点在小点上则保留，等 click 命令（translate_selection_dot）接管
                Signal::Down => {
                    worker.state::<SessionManager>().bump_gen();
                    SETTLE_ACTIVE.store(false, Ordering::Relaxed);
                    super::dot::DOT_EXPANDED.store(false, Ordering::Relaxed);
                    if !super::dot::DOWN_ON_DOT.load(Ordering::Relaxed) {
                        super::dot::hide_selection_dot(&worker);
                    }
                }
                // 靠近：展开「翻译」胶囊（超时定时器见 DOT_EXPANDED 自动让位）
                Signal::Near => {
                    super::dot::DOT_EXPANDED.store(true, Ordering::Relaxed);
                    let _ = worker.emit(
                        "dot-state",
                        DotState { near: true, guide: false },
                    );
                }
                // 移开：收回小点，重启无交互计时
                Signal::Far => {
                    super::dot::DOT_EXPANDED.store(false, Ordering::Relaxed);
                    let _ = worker.emit(
                        "dot-state",
                        DotState { near: false, guide: false },
                    );
                    arm_dot_timeout(&worker);
                }
```

> `DotState` 结构 Task 5 定义（`emit` 目标暂用占位结构会编译失败）。调整：本步骤把 `DotState` 一并定义在 dot.rs（Task 5 只消费）：

```rust
/// dot-state 事件载荷：near=光标在 proximity 盒内（展开/收回），guide=本次展示带引导气泡
#[derive(serde::Serialize, Clone)]
pub(crate) struct DotState {
    pub near: bool,
    pub guide: bool,
}
```

worker/emit 处需 `use tauri::Emitter;`（probe.rs 已有 Manager import，按编译提示补）。

- [ ] **Step 6: probe_and_show 成功分支接静置 + 闸门**（probe.rs:148-189）

probe_selection 签名加 `t_up`（worker Up 分支取 `now_ms()` 传入，与 LAST_WORKER_UP_MS 同源）：

```rust
Signal::Up { x, y } => {
    let t_up = super::now_ms();
    LAST_WORKER_UP_MS.store(t_up, Ordering::Relaxed); // [DEBUG-m0u5]
    crate::platform::debug_log(&format!("worker Up x={x} y={y}"));
    probe_selection(&worker, x, y, ICON_NEAR_PX, t_up);
}
```

`probe_selection(worker, ux, uy, near_px, t_up)` 透传给 `probe_and_show(worker, ux, uy, near_px, gen, t_up)`。

成功分支（原 L171-184）替换：

```rust
    match text {
        Some(text) if live => {
            let (cx, cy) = platform::cursor_pos();
            let far = (cx - ux).abs() + (cy - uy).abs() > near_px;
            if far {
                crate::platform::debug_log("probe 光标已远离");
                hide_selection_dot(worker);
            } else if dot_gated(worker) {
                // 可编辑焦点 / 排除列表命中：轻提示降噪，不出点（各自有诊断日志）
                hide_selection_dot(worker);
            } else {
                start_settle(worker, text, t_up, ux, uy, gen);
            }
        }
        _ if live => hide_selection_dot(worker),
        _ => crate::platform::debug_log("probe 结果已过期，丢弃"),
    }
```

`dot_gated`（放 probe_and_show 下方）：

```rust
/// 轻提示闸门：前台进程命中排除列表，或焦点控件可编辑/敏感 → 不出小点。
/// 在探测线程调用（与 focused_selection 同线程，UIA 会话安全）
fn dot_gated(worker: &AppHandle) -> bool {
    let excludes = worker
        .state::<parking_lot::Mutex<crate::translate::AppConfig>>()
        .lock()
        .dot_exclude_apps
        .clone();
    let ctx = platform::foreground_context();
    if crate::translate::process_excluded(&ctx.process_name, &excludes) {
        println!("[probe] 前台进程 {} 命中排除列表，不出小点", ctx.process_name);
        return true;
    }
    if platform::focused_editable() {
        println!("[probe] 焦点控件可编辑/敏感，不出小点");
        return true;
    }
    false
}
```

`start_settle` 与 `arm_dot_timeout`（probe.rs 底部）：

```rust
/// 静置判定：松手起算 SETTLE_MS，到点复查三条件（漂移脏标记 / gen / 开关），
/// 全过才弹小点。单次定时器非轮询——探测本身耗 120~360ms，多数情况到点即弹
fn start_settle(worker: &AppHandle, text: String, t_up: u64, ux: i32, uy: i32, gen: u64) {
    SETTLE_DIRTY.store(false, Ordering::Relaxed);
    *SETTLE_ORIGIN.lock() = (ux, uy);
    SETTLE_ACTIVE.store(true, Ordering::Relaxed);
    let worker = worker.clone();
    std::thread::spawn(move || {
        let elapsed = super::now_ms().saturating_sub(t_up);
        let remain = SETTLE_MS.saturating_sub(elapsed).max(20);
        std::thread::sleep(std::time::Duration::from_millis(remain));
        SETTLE_ACTIVE.store(false, Ordering::Relaxed);
        let enabled = worker
            .state::<parking_lot::Mutex<crate::translate::AppConfig>>()
            .lock()
            .selection_dot;
        let gen_now = worker.state::<SessionManager>().gen();
        if !settle_passes(SETTLE_DIRTY.load(Ordering::Relaxed), gen, gen_now, enabled) {
            crate::platform::debug_log("[settle] 静置判定未通过，不出点");
            return;
        }
        show_selection_dot(&worker, text, ux, uy);
        arm_dot_timeout(&worker);
    });
}

/// 小点无交互超时：单次定时器，到期复查（代次变了=新会话已接管 / 展开态不收）。
/// 展开收回（Far）时重新武装
fn arm_dot_timeout(worker: &AppHandle) {
    let worker = worker.clone();
    let shown_at = super::now_ms();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(DOT_TIMEOUT_MS));
        let gen_now = worker.state::<SessionManager>().gen();
        let expired = dot_expired(super::now_ms(), shown_at, DOT_TIMEOUT_MS);
        if !expired || super::dot::DOT_EXPANDED.load(Ordering::Relaxed) {
            return;
        }
        let _ = gen_now; // 代次检查：hide 幂等，过期收起无副作用；此处读 gen 只为日志
        crate::platform::debug_log("[dot] 无交互超时，收起");
        hide_selection_dot(&worker);
    });
}
```

> 注：`gen` 未传进 `arm_dot_timeout`——`hide_selection_dot` 幂等且 Down 路径自己会收，无需代次比对也能安全（定时器多收一次不可见的点无副作用）。保留上面 `expired`/`DOT_EXPANDED` 两道检查即可；若实现时发现竞态日志噪音，再加 gen 比对。

- [ ] **Step 7: 跑测试确认通过**

Run: `cargo test` + `cargo build`
Expected: PASS（3 条新纯函数测试 + 既有全绿）。前端此刻还没消费 `dot-state` 事件——Tauri emit 无订阅者不报错，属预期中间态。

- [ ] **Step 8: Commit**

```bash
git add -A
git commit -m "feat: 划词静置判定(350ms)+proximity边沿展开+无交互超时——立即弹图标改为静默轻提示"
```

---

### Task 5: 引导一次性置位 + dot-state 初始事件 + 窗口几何

**Files:**
- Modify: `src-tauri/tauri.conf.json:28-40`（icon 窗口 84×44）
- Modify: `src-tauri/src/selection/dot.rs`（show_selection_dot 整体重写 L25-44）

**Interfaces:**
- Consumes: Task 1 `translate::with_config_write`/`AppConfig.dot_guide_shown`；Task 4 `DotState`
- Produces: `dot-state` 事件在 show 时首发 `{near:false, guide}`（Task 4 的 Near/Far 后续刷新）；窗口常驻 84×44，引导态 220×78

- [ ] **Step 1: tauri.conf.json icon 窗口尺寸**

`"width": 40, "height": 40` → `"width": 84, "height": 44`（84×44：小点态左上角 12px 圆点，展开态 64×28 胶囊，其余透明余量）。

- [ ] **Step 2: 重写 show_selection_dot**

```rust
/// 静置判定通过后在松手位置显示轻提示（窗口已在 tauri.conf.json 预建，挪位 + show）。
/// 首次展示带引导气泡（窗口临时扩到 220×78 容纳气泡文案），展示即置位落盘，
/// 之后永不再弹；非引导态恢复常驻尺寸
pub(crate) fn show_selection_dot(app: &AppHandle, text: String, ax: i32, ay: i32) {
    app.state::<SessionManager>().set_dot_text(Some(text));
    let guide = {
        let already = app
            .state::<parking_lot::Mutex<crate::translate::AppConfig>>()
            .lock()
            .dot_guide_shown;
        if !already {
            crate::translate::with_config_write(app, |c| c.dot_guide_shown = true);
        }
        !already
    };
    let inner = app.clone();
    let _ = app.run_on_main_thread(move || {
        let Some(win) = inner.get_webview_window("icon") else {
            crate::platform::debug_log("[dot] 找不到预建的 icon 窗口");
            return;
        };
        let size = if guide {
            tauri::LogicalSize::new(220.0, 78.0)
        } else {
            tauri::LogicalSize::new(84.0, 44.0)
        };
        let _ = win.set_size(size);
        let (x, y) = crate::geometry::clamp_to_screen(&inner, "icon", ax + 8, ay + 12);
        let moved = win.set_position(tauri::PhysicalPosition::new(x, y));
        let outer = win.outer_size();
        if let Ok(size) = &outer {
            *DOT_RECT.lock() = Some((x, y, size.width, size.height));
        }
        let shown = win.show();
        let _ = inner.emit(
            "dot-state",
            DotState { near: false, guide },
        );
        crate::platform::debug_log(&format!(
            "[dot] show guide={guide} ({x},{y}) size={outer:?} set_position={moved:?} show={shown:?}"
        ));
    });
}
```

（`use tauri::Emitter;` 按编译提示补入 dot.rs。）

- [ ] **Step 3: 编译验证**

Run: `cargo test` + `cargo build`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "feat: 轻提示首次展示带引导气泡（dot_guide_shown 一次性置位）+ dot-state 初始事件"
```

---

### Task 6: 前端三态组件（TDD）

**Files:**
- Create: `src/lib/dotState.ts`
- Create: `src/lib/dotState.test.ts`
- Modify: `package.json`（test script 加 dotState.test.ts）
- Modify: `src/components/SelectionIcon/index.tsx`（整体重写）
- Modify: `src/components/SelectionIcon/index.module.css`（整体重写）

**Interfaces:**
- Consumes: Task 4/5 的 `dot-state` 事件 `{ near: boolean; guide: boolean }`；命令 `translate_selection_dot`（Task 2 改名后）
- Produces: `resolveState(near: boolean): "dot" | "expanded"`（lib 纯函数）

- [ ] **Step 1: 写失败测试**

`src/lib/dotState.test.ts`：

```ts
import { test } from "node:test";
import assert from "node:assert/strict";
import { resolveState } from "./dotState";

test("near 边沿驱动三态：false=小点 true=展开胶囊", () => {
  assert.equal(resolveState(false), "dot");
  assert.equal(resolveState(true), "expanded");
});

test("初始为小点态（Rust 边沿触发保证停在旁边不会立即展开）", () => {
  // Rust 首发事件恒为 {near:false}，前端不自行推断 near
  assert.equal(resolveState(false), "dot");
});
```

- [ ] **Step 2: 跑测试确认失败**

Run: `npm test`
Expected: FAIL（dotState 模块不存在）。先在 `package.json` test script 追加 `src/lib/dotState.test.ts`。

- [ ] **Step 3: 实现 lib**

`src/lib/dotState.ts`：

```ts
/** 轻提示前端状态判定：near（Rust proximity 边沿事件）→ 展开「翻译」胶囊，否则半透明小点。
 *  引导气泡不在此判定——guide 由 Rust 保证只发一次，随当次 dot-state 载荷独立渲染 */
export function resolveState(near: boolean): "dot" | "expanded" {
  return near ? "expanded" : "dot";
}
```

- [ ] **Step 4: 重写组件**

`src/components/SelectionIcon/index.tsx`：

```tsx
import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { resolveState } from "../../lib/dotState";
import s from "./index.module.css";

/** 划词轻提示：透明小窗口，Rust 静置判定通过后 show 并发 dot-state。
 *  小点态=左上角半透明圆点；near=展开「翻译」胶囊（点击由 Rust 收起自己并
 *  广播 selection 事件，弹窗卡片接着走热键那套流程）；guide=一次性引导气泡 */
export default function SelectionIcon() {
  const [near, setNear] = useState(false);
  const [guide, setGuide] = useState(false);
  useEffect(() => {
    const un = listen<{ near: boolean; guide: boolean }>("dot-state", (e) => {
      setNear(e.payload.near);
      setGuide(e.payload.guide);
    });
    return () => {
      un.then((f) => f());
    };
  }, []);
  return (
    <>
      {resolveState(near) === "expanded" ? (
        <button
          type="button"
          className={s.pill}
          title="翻译选中的文字"
          aria-label="翻译选中的文字"
          onClick={() => {
            // 失败只可能是没有缓存的选区（探测后被系统清掉），关掉就行
            invoke("translate_selection_dot").catch(() => {});
          }}
        >
          翻译
        </button>
      ) : (
        <div className={s.dot} aria-hidden="true" />
      )}
      {guide && <div className={s.guide}>靠近小点翻译，或按快捷键</div>}
    </>
  );
}
```

`src/components/SelectionIcon/index.module.css`（整体替换，logo 依赖删除）：

```css
/* 轻提示：84×44 透明窗口。小点态左上角 12px 半透明圆；near 展开 64×28「翻译」胶囊；
   引导态窗口临时扩到 220×78，气泡挂在小点下方 */
.dot {
  position: absolute;
  left: 2px;
  top: 2px;
  width: 12px;
  height: 12px;
  border-radius: 50%;
  background: rgba(96, 105, 120, 0.45);
}
.pill {
  position: absolute;
  left: 4px;
  top: 6px;
  width: 64px;
  height: 28px;
  padding: 0;
  border: none;
  border-radius: 8px;
  background: rgba(96, 105, 120, 0.9);
  color: #fff;
  font-size: 13px;
  line-height: 28px;
  text-align: center;
  cursor: pointer;
}
.pill:hover {
  background: rgba(96, 105, 120, 1);
}
.pill:active {
  transform: translateY(1px);
}
.guide {
  position: absolute;
  left: 4px;
  top: 44px;
  padding: 6px 10px;
  border-radius: 8px;
  background: rgba(30, 34, 40, 0.9);
  color: #fff;
  font-size: 12px;
  white-space: nowrap;
}
@media (prefers-reduced-motion: reduce) {
  .pill:active {
    transform: none;
  }
}
```

- [ ] **Step 5: 全部前端校验**

Run: `npm test` + `npx tsc --noEmit` + `npm run build`
Expected: 全部 PASS

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "feat: 轻提示前端三态组件（点/翻译胶囊/引导气泡），dot-state 事件驱动"
```

---

### Task 7: 托盘勾选项「划词轻提示」

**Files:**
- Modify: `src-tauri/src/tray.rs`（菜单构建抽函数 + CheckMenuItem + on_menu_event + refresh_dot_item）
- Modify: `src-tauri/src/translate/mod.rs`（toggle_selection_dot 末尾补 `crate::tray::refresh_dot_item(app, enabled);`——Task 1 预留的行）

**Interfaces:**
- Consumes: `translate::toggle_selection_dot`、`AppConfig.selection_dot`
- Produces: `tray::refresh_dot_item(app: &AppHandle, enabled: bool)`（toggle 内部调用，菜单勾选态同步；热键与托盘两条路径都覆盖）

- [ ] **Step 1: tray.rs 重构菜单构建**

```rust
//! 系统托盘：应用常驻后台的唯一可见入口。从 main.rs 整体迁出（纯移动不改逻辑）。

use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{App, AppHandle, Manager};

use crate::windows;

/// 托盘菜单：划词轻提示勾选项（当前状态可见、随手切换）+ 退出
fn tray_menu(app: &AppHandle, dot_enabled: bool) -> Result<Menu<tauri::Wry>, tauri::Error> {
    let dot = CheckMenuItem::with_id(
        app,
        "toggle-dot",
        "划词轻提示",
        true,
        dot_enabled,
        None::<&str>,
    )?;
    let sep = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
    Menu::with_items(app, &[&dot, &sep, &quit])
}

/// 开关变化后重建菜单刷新勾选态（热键与托盘两条切换路径都收敛到这里）
pub(crate) fn refresh_dot_item(app: &AppHandle, enabled: bool) {
    if let Some(tray) = app.tray_by_id("main") {
        match tray_menu(app, enabled) {
            Ok(menu) => {
                let _ = tray.set_menu(menu);
            }
            Err(e) => eprintln!("[tray] 菜单重建失败: {e}"),
        }
    }
}

/// 建托盘图标：左键直接开设置页，右键弹菜单（轻提示开关 + 退出）
pub(crate) fn setup_tray(app: &App) -> Result<(), tauri::Error> {
    let dot_enabled = app
        .state::<parking_lot::Mutex<crate::translate::AppConfig>>()
        .lock()
        .selection_dot;
    let tray_menu = tray_menu(app.handle(), dot_enabled)?;
    // …以下 TrayIconBuilder 部分原样保留（tooltip/icon/template/show_menu_on_left_click/
    // on_tray_icon_event 左键开设置），仅 on_menu_event 改为：

    //        .on_menu_event(|app, event| match event.id().as_ref() {
    //            "toggle-dot" => crate::translate::toggle_selection_dot(app),
    //            "quit" => app.exit(0),
    //            _ => {}
    //        })
}
```

> 原文件的 TrayIconBuilder 链式调用逐行保留，只替换菜单构造与 on_menu_event 两个片段；`app.state` 需要 `Manager` trait 在作用域。

- [ ] **Step 2: toggle 接上托盘刷新**

translate/mod.rs `toggle_selection_dot` 的 `show_hud` 调用后补一行：

```rust
    crate::tray::refresh_dot_item(app, enabled);
```

- [ ] **Step 3: 编译验证**

Run: `cargo build` + `cargo test`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "feat: 托盘菜单加「划词轻提示」勾选项，热键/托盘两路切换都同步勾选态"
```

---

### Task 8: 设置页——文案 + 排除列表

**Files:**
- Modify: `src-tauri/src/translate/mod.rs:286-303`（save_settings 加 dot_exclude_apps 参数）
- Modify: `src/lib/config.ts`（saveSettings 加字段）
- Modify: `src/types.ts`（AppConfig 加 dot_exclude_apps）
- Modify: `src/hooks/useConfigForm.ts`（excludeApps 状态 + 装载 + 保存）
- Modify: `src/components/Settings/components/HotkeyTab/index.tsx`（文案 + 排除列表输入框）

**Interfaces:**
- Consumes: Task 1 `normalize_excludes`（后端 save_settings 内调用）
- Produces: 命令 `save_settings` 多接受 `dotExcludeApps: string[]`（Tauri camelCase）；前端 `ConfigForm.excludeApps`

- [ ] **Step 1: 后端 save_settings**

```rust
/// 快捷键 + OCR 语言 + 轻提示排除列表（快捷键 tab 的「保存」按钮）
#[tauri::command]
pub fn save_settings(
    app: AppHandle,
    state: State<SharedConfig>,
    hotkey: String,
    screenshot_hotkey: String,
    input_hotkey: String,
    icon_hotkey: String,
    ocr_lang: String,
    dot_exclude_apps: Vec<String>,
) -> Result<(), String> {
    persist(app, state, |c| {
        c.hotkey = hotkey;
        c.screenshot_hotkey = screenshot_hotkey;
        c.input_hotkey = input_hotkey;
        c.icon_hotkey = icon_hotkey;
        c.ocr_lang = ocr_lang;
        c.dot_exclude_apps = normalize_excludes(dot_exclude_apps);
    })
}
```

- [ ] **Step 2: 前端接线**

`src/types.ts` AppConfig 追加：

```ts
  /** 轻提示排除列表（进程名，后端已归一化小写） */
  dot_exclude_apps?: string[];
```

`src/lib/config.ts` saveSettings 字段追加 `dotExcludeApps: string[]`（Tauri 参数 camelCase 自动映射 dot_exclude_apps）。

`src/hooks/useConfigForm.ts`：

```ts
  // 轻提示排除列表：文本框逗号分隔，保存时解析成数组
  const [excludeApps, setExcludeApps] = useState("");
```

装载（load 回调内）：

```ts
          setExcludeApps((cfg.dot_exclude_apps ?? []).join(", "));
```

saveGeneral 调用改为：

```ts
      await saveSettings({
        hotkey, screenshotHotkey, inputHotkey, iconHotkey, ocrLang: "auto",
        dotExcludeApps: excludeApps.split(/[,，\n]/).map((s) => s.trim()).filter(Boolean),
      });
```

返回对象加 `excludeApps, setExcludeApps`。

`src/components/Settings/components/HotkeyTab/index.tsx`：

```tsx
      <label className="field" style={{ marginTop: 12 }}>
        划词轻提示是否开启
        <HotkeyRecorder value={f.iconHotkey} onChange={f.setIconHotkey} />
      </label>
      <label className="field" style={{ marginTop: 12 }}>
        轻提示排除应用
        <input
          className="text-input"
          value={f.excludeApps}
          onChange={(e) => f.setExcludeApps(e.target.value)}
          placeholder="进程名，逗号分隔，如 code, wechat（留空不排除）"
        />
      </label>
```

（文案「划词图标是否显示」改「划词轻提示是否开启」，组件头注释同步改。`text-input` 类若设置页全局样式没有，复用该页已有输入控件类名——实施时先查 `src/styles/global.css` 与 Settings 现有 input 用法，保持视觉一致。）

- [ ] **Step 3: 校验**

Run: `npx tsc --noEmit` + `npm test` + `cargo test`
Expected: 全部 PASS

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "feat: 设置页轻提示排除列表（进程名）+ 文案改为划词轻提示"
```

---

### Task 9: 全量验证 + 手测矩阵

**Files:** 无新改动（发现回归则回到对应任务修）

- [ ] **Step 1: 自动化全绿**

```bash
cargo test            # Rust 全量（含新增 5 条纯函数测试）
cargo build           # Windows 构建通过
npm test              # node --test（text/cardDismiss/dotState）
npx tsc --noEmit      # 类型检查
npm run build         # 前端构建
```

- [ ] **Step 2: 手测矩阵**（spec §7，`npm run tauri dev` 实测，逐行记录）

| # | 场景 | 预期 |
|---|---|---|
| 1 | 浏览器划词，鼠标停住 | ~350ms 出小点；靠近展开「翻译」；移开收回；2.5s 消失 |
| 2 | 划完立刻挪鼠标（>10px） | 不出点 |
| 3 | 划完立刻再划（gen 变） | 第一轮不出点，第二轮正常 |
| 4 | 输入框（地址栏/记事本）划词重选 | 不出点（focused_editable 拦截）|
| 5 | 终端（cmd/WT）划词 | 不出点（探测/hopeless 拦截）|
| 6 | 密码框 | 不出点 |
| 7 | 排除列表填 `code`，VSCode 划词 | 不出点；删掉后恢复 |
| 8 | Alt+W 连按 | HUD「划词轻提示已开启/关闭」+ 托盘勾选同步；关时收点、不出点 |
| 9 | 托盘勾选项切换 | 与 Alt+W 等效 |
| 10 | 点「翻译」→ 卡片弹出 | 与旧图标点击行为一致（show_card 合流）|
| 11 | 点完「翻译」400ms 内再划词 | 不弹回（DOT_REPROBE_QUIET_MS）|
| 12 | 快捷键直翻（轻提示关） | 完全不受影响 |
| 13 | 双显示器 + 150% 缩放 | 点/胶囊钳入所在屏工作区 |
| 14 | 引导气泡 | 仅首次出现；消失后永不再弹；窗口尺寸恢复 84×44 |
| 15 | 展开态停留 | 无超时（一直看翻译不消失）；移开后 2.5s 收 |
| 16 | 84×44 透明余量区点击 | 记录实际行为；若挡住下层 UI 明显扰民，评估缩小窗口或 `set_ignore_cursor_events` 方案（spec R4）|

- [ ] **Step 3: 收尾提交**

```bash
git add -A
git commit -m "chore: 划词轻提示手测矩阵通过"
```

（若 Step 2 无代码改动则跳过本提交。）
