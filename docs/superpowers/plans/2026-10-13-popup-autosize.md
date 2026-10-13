# 弹窗自适应高度 实施计划（issue #2 第 2 条）

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 卡片高度跟着内容走——内容少时窗口收缩到刚好包住卡片，不再有下方大片空白；透明区域不再吞掉下层窗口的点击。

**Architecture:** 前端在一个专用容器（`.card-body`）里量出内容自然高度（该容器不受 `.card{height:100%}` 影响），把高度交给新命令 `fit_window`；Rust 负责算最终尺寸（下限 60px、上限所在屏工作区 70%、扣除标题栏）与位置（复用 `clamp_to_screen` 的钳制逻辑），一次性 `set_size` + `set_position`。

**Tech Stack:** Tauri 2（`tauri::LogicalSize`、`WebviewWindow::set_size/inner_size/outer_size`）、React 18（callback ref + `ResizeObserver`）、现有 `src/style.css`。

**Spec:** `https://github.com/mrzym99/Wordpeek/issues/2` 第 2 条；设计决策与取舍见本文「设计决策」章。

## 设计决策

| 决策 | 选择 | 理由 / 否决的替代方案 |
|---|---|---|
| 收缩方式 | 真正缩小窗口，而不是只让 `.card` 自适应内容 | 窗口 `transparent: true` 且始终置顶：卡片视觉变小后，剩余窗口区域仍然命中鼠标——点击会落在那块看不见的窗口上，既不激活下层应用，也不会触发 `blur` 自动隐藏（现有 `tauri://blur` → hide 逻辑失效）。只改 CSS 会留下这个坑 |
| 尺寸由谁算 | Rust 算，前端只上报内容高度 | `clamp_to_screen`（现成）、`work_area_at`（现成）都在 Rust；两端各写一套屏幕数学必然漂移 |
| 上限 | 所在屏**工作区**高度的 70%，超出则卡片内部滚动（`.card{overflow-y:auto}` 已有） | 工作区已排除任务栏，不会压到任务栏；70% 保留上下文，不遮满屏 |
| 宽度 | 保持当前宽度不变 | 窗口 `resizable: true`，长文本用户可手动拉宽；若每次 fit 都把宽度重置回 400 会抢走这个能力 |
| 测量对象 | 新增 `.card-body` 包裹层 | `.card{height:100%}` 随窗口变化，量它永远等于窗口高度（循环依赖）；子元素溢出父级时 `offsetHeight` 仍是自然高度 |
| 测量时机 | `ResizeObserver` 回调里直接调用（不用 `requestAnimationFrame`） | 弹窗在显示前是 `visible:false`，隐藏窗口里 rAF 会被节流甚至不触发，可能导致首次尺寸永远不上报 |
| 标题栏 | 用 `outer_size().height - inner_size().height` 扣掉 | Tauri 的 `set_size` 走 tao 的 `set_inner_size`（已核实 tauri-runtime-wry 2.12 `WindowMessage::SetSize`），设的是**客户区**高度；带标题栏的窗口（后续 issue #2 第 1 条的输入翻译窗口）上限必须扣掉标题栏 |
| 显示时机 | 维持现状：`show()` 立即执行，内容渲染后 fit | 让「等 fit 完再 show」需要隐藏窗口里跑通量测链路，风险大于收益（卡片会先按「翻译中…」高度收缩、再随结果撑开，肉眼是 1 帧的变化） |

## Global Constraints

- 不新增任何依赖（Cargo 与 npm 均无新增）
- `.card` 的直接子元素里，`.drag-strip` 保持在最前（`position: fixed`，不参与高度计算）；其余内容全部放进 `.card-body`
- 窗口配置（`transparent` / `alwaysOnTop` / `resizable` / 初始 `width: 400`）与 `src-tauri/capabilities/default.json` 都不改（`fit_window` 是自定义命令，不需要 core 窗口权限）
- 注释与用户可见文案一律中文；禁止 TODO/TBD
- Rust 任务收尾必须 `cd src-tauri && cargo test` 通过；前端任务收尾必须 `npx tsc --noEmit` 通过
- 提交信息中文，格式 `feat|fix|test|docs: 描述`

---

### Task 1: Rust —— 尺寸钳制纯函数 + `fit_window` 命令

**Files:**
- Modify: `src-tauri/src/main.rs`（`clamp_to_screen` 抽出纯函数并新增 label 参数；新增 `fit_height`、`monitor_rect_at`、`fit_window`；`invoke_handler` 注册 `fit_window`）
- Test: `src-tauri/src/main.rs` 的 `mod tests`

**Interfaces:**
- Consumes: 现有 `work_area_at(x, y) -> (i32, i32, i32, i32)`（Win32 工作区，物理像素，失败返回全 0）、`app.get_webview_window(label)`
- Produces:
  - `fn fit_height(content_h: f64, work_h: i32, chrome_h: f64) -> f64`
  - `fn clamp_rect_to_monitor(monitor: (i32, i32, i32, i32), win_w: i32, win_h: i32, x: i32, y: i32) -> (i32, i32)`
  - `fn monitor_rect_at(win: &tauri::WebviewWindow, x: i32, y: i32) -> Option<(i32, i32, i32, i32)>`
  - `fn clamp_to_screen(app: &AppHandle, label: &str, x: i32, y: i32) -> (i32, i32)`（**签名变更**：加 `label`）
  - 命令 `fit_window(app: AppHandle, label: String, height: f64) -> Result<(), String>`

- [ ] **Step 1: 写失败测试**

在 `src-tauri/src/main.rs` 的 `mod tests` 里追加：

```rust
    #[test]
    fn fit_height_clamps_between_floor_and_work_area() {
        // 工作区 1040px 高：上限 = 728px（70%）
        assert_eq!(fit_height(120.0, 1040, 0.0), 120.0); // 常见短卡片：原样
        assert_eq!(fit_height(10.0, 1040, 0.0), 60.0); // 下限，别只剩一条缝
        assert_eq!(fit_height(900.0, 1040, 0.0), 728.0); // 超出上限 → 内部滚动
        assert_eq!(fit_height(900.0, 1040, 31.0), 697.0); // 有标题栏：上限再扣掉标题栏
        assert_eq!(fit_height(900.0, 0, 0.0), 800.0); // 拿不到工作区 → 退回 800
        assert_eq!(fit_height(9000.0, 50, 31.0), 60.0); // 工作区极小时不低于下限
    }

    #[test]
    fn clamp_rect_to_monitor_keeps_window_inside() {
        // 主屏 1920×1080，窗口 400×300
        assert_eq!(clamp_rect_to_monitor((0, 0, 1920, 1080), 400, 300, -50, -20), (0, 0));
        assert_eq!(clamp_rect_to_monitor((0, 0, 1920, 1080), 400, 300, 1900, 1070), (1520, 780));
        assert_eq!(clamp_rect_to_monitor((0, 0, 1920, 1080), 400, 300, 500, 500), (500, 500));
        // 副屏在左侧（负坐标），窗口比屏还宽时贴左上角而不是弹出去
        assert_eq!(clamp_rect_to_monitor((-1920, 0, 1920, 1080), 400, 300, -1900, 10), (-1900, 10));
        assert_eq!(clamp_rect_to_monitor((-1920, 0, 1920, 1080), 2400, 300, -1900, 10), (-1920, 10));
    }
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cd src-tauri && cargo test fit_height_clamps_between_floor_and_work_area clamp_rect_to_monitor_keeps_window_inside`

Expected: 编译失败，`cannot find function fit_height` / `cannot find function clamp_rect_to_monitor`

- [ ] **Step 3: 实现纯函数，并把 `clamp_to_screen` 改成复用它们**

在 `clamp_to_screen` 上方插入：

```rust
/// 内容高度 → 窗口客户区（inner）逻辑高度：下限 60px，
/// 上限为工作区高度的 70% 再扣掉标题栏（拿不到工作区时退回 800px）。
/// 上限先算再取下限兜底：极小工作区也不会算出上限小于下限而 panic。
fn fit_height(content_h: f64, work_h: i32, chrome_h: f64) -> f64 {
    let max = if work_h > 0 {
        work_h as f64 * 0.7 - chrome_h
    } else {
        800.0
    };
    content_h.clamp(60.0, max.max(60.0))
}

/// 把「左上角在 (x, y)、尺寸 win_w×win_h」的窗口钳进显示器矩形内。
/// 窗口比屏还大时贴左上角（不弹到屏外）。
fn clamp_rect_to_monitor(
    monitor: (i32, i32, i32, i32),
    win_w: i32,
    win_h: i32,
    x: i32,
    y: i32,
) -> (i32, i32) {
    let (mx, my, mw, mh) = monitor;
    let cx = x.clamp(mx, (mx + mw - win_w).max(mx));
    let cy = y.clamp(my, (my + mh - win_h).max(my));
    (cx, cy)
}

/// 找到包含点 (x, y) 的显示器矩形（物理像素）。查不到显示器信息时返回 None。
fn monitor_rect_at(win: &tauri::WebviewWindow, x: i32, y: i32) -> Option<(i32, i32, i32, i32)> {
    let monitors = win.available_monitors().ok()?;
    let m = monitors.into_iter().find(|m| {
        let p = m.position();
        let s = m.size();
        x >= p.x && x < p.x + s.width as i32 && y >= p.y && y < p.y + s.height as i32
    })?;
    let p = m.position();
    let s = m.size();
    Some((p.x, p.y, s.width as i32, s.height as i32))
}
```

把 `clamp_to_screen` 的函数体替换为（逻辑等价，只是抽出纯函数 + 加 label 参数）：

```rust
/// 把弹窗位置钳制在包含该点的屏幕内，避免边缘选词时弹窗伸出屏幕外。
/// 任一步骤拿不到信息（找不到窗口/显示器）就原样返回，不钳制。
fn clamp_to_screen(app: &AppHandle, label: &str, x: i32, y: i32) -> (i32, i32) {
    let Some(win) = app.get_webview_window(label) else {
        return (x, y);
    };
    let Some(rect) = monitor_rect_at(&win, x, y) else {
        return (x, y);
    };
    let Ok(win_size) = win.outer_size() else {
        return (x, y);
    };
    let (cx, cy) = clamp_rect_to_monitor(
        rect,
        win_size.width as i32,
        win_size.height as i32,
        x,
        y,
    );
    println!("[pos] cursor ({x}, {y}) -> clamped ({cx}, {cy}); monitor {rect:?}, window {win_size:?}");
    (cx, cy)
}
```

同时改调用点：`handle_hotkey_trigger` 里的

```rust
let (x, y) = clamp_to_screen(&app, cx + 12, cy + 16);
```

改为

```rust
let (x, y) = clamp_to_screen(&app, "main", cx + 12, cy + 16);
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cd src-tauri && cargo test`

Expected: 全部 PASS（含原有 `parse_common_hotkeys`）

- [ ] **Step 5: 实现 `fit_window` 命令并注册**

在 `clamp_to_screen` 之后插入：

```rust
/// 前端量好卡片内容高度后调用：把窗口调整到刚好包住卡片，并重新钳制位置。
/// 只改高度——宽度保持用户当前宽度（长文本可手动拉宽）。
/// 位置钳制用「我们自己算出的新外框尺寸」，不依赖 set_size 之后立刻回读（那是异步消息）。
#[tauri::command]
fn fit_window(app: AppHandle, label: String, height: f64) -> Result<(), String> {
    let win = app
        .get_webview_window(&label)
        .ok_or_else(|| format!("窗口不存在: {label}"))?;
    let scale = win.scale_factor().map_err(|e| e.to_string())?;
    let inner = win.inner_size().map_err(|e| e.to_string())?;
    let outer = win.outer_size().map_err(|e| e.to_string())?;
    let pos = win.outer_position().map_err(|e| e.to_string())?;
    // 标题栏/边框占掉的像素（逻辑单位）：无边框弹窗为 0，带标题栏的窗口约 31px
    let chrome_w = (outer.width.saturating_sub(inner.width)) as f64 / scale;
    let chrome_h = (outer.height.saturating_sub(inner.height)) as f64 / scale;
    // 高度上限用工作区（排除任务栏）；拿不到工作区则退回该屏整块矩形
    let (_, _, _, work_h) = work_area_at(pos.x + 8, pos.y + 8);
    let inner_h = fit_height(height, work_h, chrome_h);
    let inner_w = inner.width as f64 / scale;
    win.set_size(tauri::LogicalSize::new(inner_w, inner_h))
        .map_err(|e| e.to_string())?;

    // 新外框尺寸：客户区 + 边框/标题栏
    let new_w = (inner_w + chrome_w).round().max(1.0) as i32;
    let new_h = (inner_h + chrome_h).round().max(1.0) as i32;
    let (nx, ny) = match monitor_rect_at(&win, pos.x + 8, pos.y + 8) {
        Some(rect) => clamp_rect_to_monitor(rect, new_w, new_h, pos.x, pos.y),
        None => (pos.x, pos.y),
    };
    win.set_position(tauri::PhysicalPosition::new(nx, ny))
        .map_err(|e| e.to_string())?;
    println!("[fit] {label} content {height} -> inner {inner_w}x{inner_h}, pos ({nx}, {ny})");
    Ok(())
}
```

在 `main()` 的 `invoke_handler` 里，`screenshot_cancel,` 之后追加一行 `fit_window,`：

```rust
        .invoke_handler(tauri::generate_handler![
            open_settings,
            get_update_status,
            check_update_now,
            install_update,
            screenshot_finish,
            screenshot_cancel,
            fit_window,
            copy_text,
            translate::translate,
            translate::get_config,
            translate::save_config
        ])
```

- [ ] **Step 6: 跑测试确认通过**

Run: `cd src-tauri && cargo test`

Expected: 全部 PASS（`fit_window` 是命令，靠 Step 7 的实机验证）

- [ ] **Step 7: 提交**

```bash
git add src-tauri/src/main.rs
git commit -m "feat: 新增 fit_window 命令，弹窗按内容高度调整尺寸并重新钳制位置"
```

---

### Task 2: 前端 —— 内容容器 + 自动上报高度

**Files:**
- Create: `src/lib/autofit.ts`
- Modify: `src/components/Popup.tsx`（划词卡片包 `.card-body`）
- Modify: `src/components/ShotCard.tsx`（截图卡片包 `.card-body`）

**Interfaces:**
- Consumes: Task 1 的命令 `fit_window(label, height)`
- Produces: `useAutoFitWindow(label: string, chrome?: number): (el: HTMLDivElement | null) => void`（返回值直接用作卡片内容容器的 `ref`）

- [ ] **Step 1: 写 hook**

创建 `src/lib/autofit.ts`：

```ts
import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

/** 卡片上下 padding（16×2）+ 边框（1×2）+ 1px 取整余量 */
const CARD_CHROME = 35;
/** 高度变化小于该值不上报，避免来回抖动 */
const EPSILON = 3;

/**
 * 让窗口高度跟着卡片内容走：内容少时窗口收缩到刚好包住卡片，
 * 不再留下"透明但吃点击"的空白区域。返回内容容器的 ref——
 * 该容器高度即内容自然高度（不受 .card 的 height:100% 影响）。
 */
export function useAutoFitWindow(label: string, chrome: number = CARD_CHROME) {
  const [body, setBody] = useState<HTMLDivElement | null>(null);
  useEffect(() => {
    if (!body) return;
    let last = 0;
    // 不用 requestAnimationFrame：窗口显示前是隐藏状态，rAF 会被节流甚至不触发
    const apply = () => {
      const h = Math.ceil(body.offsetHeight) + chrome;
      if (Math.abs(h - last) < EPSILON) return;
      last = h;
      invoke("fit_window", { label, height: h }).catch(() => {});
    };
    const ro = new ResizeObserver(apply);
    ro.observe(body);
    apply(); // 首次测量（ResizeObserver 只保证后续变化）
    return () => ro.disconnect();
  }, [body, label, chrome]);
  return setBody;
}
```

- [ ] **Step 2: 划词卡片接入**

`src/components/Popup.tsx`：

1. 顶部 import 追加 `import { useAutoFitWindow } from "../lib/autofit";`
2. 组件内 `const [word, setWord] = useState("");` 之前加：

```tsx
  // 卡片内容高度上报给 Rust，窗口按内容收缩（防止透明区域吞掉下层点击）
  const bodyRef = useAutoFitWindow("main");
```

3. 返回的卡片结构改为（`.drag-strip` 留在 `.card` 直接子级，其余内容进 `.card-body`）：

```tsx
  return (
    <div className="card">
      {/* 顶部 padding 区是隐形拖拽热区：按住可移动窗口；窗口边缘仍可拉伸 */}
      <div className="drag-strip" data-tauri-drag-region />
      <div className="card-body" ref={bodyRef}>
        <div className="word-head">
          {/* ……保持原有内容不动…… */}
        </div>

        {error && <div className="error">{error}</div>}
        {!info && !error && <div className="loading">翻译中...</div>}

        {info && (
          <>
            {/* ……保持原有释义 / 词形变化 / 同义词渲染不动…… */}
          </>
        )}
      </div>
    </div>
  );
```

- [ ] **Step 3: 截图卡片接入**

`src/components/ShotCard.tsx`：

1. import 追加 `import { useAutoFitWindow } from "../lib/autofit";`
2. 组件函数第一行加：

```tsx
  // 与划词卡片共用 main 窗口，同样按内容高度收缩
  const bodyRef = useAutoFitWindow("main");
```

3. 返回结构改为：

```tsx
  return (
    <div className="card">
      <div className="drag-strip" data-tauri-drag-region />
      <div className="card-body" ref={bodyRef}>
        <div className="word-head">
          {/* ……保持原有内容不动…… */}
        </div>
        {shotResult.text && (
          <>
            {/* ……textarea / 按钮行保持不动…… */}
          </>
        )}
        {shotResult.error && <div className="error">{shotResult.error}</div>}
        {loading && <div className="shot-loading">翻译中…</div>}
        {shotResult.info && (
          <>
            {/* ……译文列表 / 复制译文按钮保持不动…… */}
          </>
        )}
      </div>
    </div>
  );
```

- [ ] **Step 4: 类型检查**

Run: `npx tsc --noEmit`

Expected: 无输出（无错误）

- [ ] **Step 5: 实机验证（本任务的验收，逐条记录结果）**

Run: `npm install && npm run tauri dev`

1. 划词短单词（如 `cat`）→ 卡片下沿紧贴内容，无空白
2. 打开记事本放在卡片下方，点卡片正下方的空白位置 → 应**激活/聚焦记事本**（说明空白区域已不存在）
3. 划词长词条（如 `error`，释义+同义+词形多）→ 卡片随内容撑高；超高时出现内部滚动条且高度不超过屏幕工作区 70%
4. 截图翻译（`Ctrl+Alt+S`）→ 卡片同样贴合内容；点「重新翻译」→ 高度随译文变化
5. 拖住顶部 16px 拖拽条移动卡片 → 位置正常；拖动窗口右边缘拉宽 → 宽度保持，高度仍贴合内容
6. 切到 125%/150% 缩放与副屏重复第 1、3 步 → 高度不溢出工作区、位置不越界

- [ ] **Step 6: 提交**

```bash
git add src/lib/autofit.ts src/components/Popup.tsx src/components/ShotCard.tsx
git commit -m "feat: 弹窗高度随内容自适应，消除卡片下方空白"
```

---

## 验收清单（对应 issue #2 第 2 条）

- [ ] 短内容（单词语义 1～2 条）卡片高度 ≈ 内容高度，肉眼无空白
- [ ] 长内容卡片撑高，上限为工作区 70%，超出部分内部滚动
- [ ] 卡片下方原空白区域点击可穿透到下层窗口
- [ ] 划词卡片与截图卡片两种形态都生效；「翻译中…」→ 结果切换时高度跟随
- [ ] 混合 DPI / 副屏下高度与位置都合法
- [ ] `cargo test`、`npx tsc --noEmit` 通过

## 已知取舍

- 卡片先按「翻译中…」的高度收缩、结果到达后再撑开，肉眼是 1 帧变化。若要完全无跳动，需要在隐藏窗口里先跑通量测再 `show`（rAF 在隐藏窗口不可靠），风险大于收益，暂不做。
- 用户手动拉伸的高度会被下一次内容变化覆盖（宽度不受影响）。
