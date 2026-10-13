# 输入翻译窗口 实施计划（issue #2 第 1 条）

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 增加「输入翻译」——一个可随时唤出的窗口，打字/粘贴文本后按快捷键或按钮翻译，不再只能依赖「选中文字 + 抓剪贴板」。

**Architecture:** 新增第四个常驻窗口 `input`（`?page=input`，普通带标题栏窗口，点 ✕ 只隐藏），托盘菜单「输入翻译」与新增全局快捷键 `Ctrl+Alt+I` 唤出；翻译仍走 Rust 端 `translate` 命令，只新增一个可选参数 `to`（`"zh"` / `"en"`）来支持译为英文。卡片/结果渲染抽成共享组件 `SenseList`，避免第四处复制释义渲染。

**Tech Stack:** Tauri 2（多窗口 `tauri.conf.json`、托盘菜单、global-shortcut）、React 18 + TypeScript、现有 `translate.rs` 管线。

**Spec:** `https://github.com/mrzym99/Wordpeek/issues/2` 第 1 条；依赖计划 `docs/superpowers/plans/2026-10-13-popup-autosize.md` 提供的 `fit_window` 命令与 `useAutoFitWindow`。

## 设计决策

| 决策 | 选择 | 理由 / 否决的替代方案 |
|---|---|---|
| 交互载体 | 独立窗口（`label: "input"`，普通窗口，可 Alt+Tab 回来） | 主弹窗的语义是「就地结果卡片」：失焦即隐藏、无输入焦点保持能力，做输入框必然与自动隐藏打架 |
| 唤出方式 | 托盘菜单 + 全局快捷键 `Ctrl+Alt+I`（可在设置里改） | 托盘是现有唯一的常驻入口；快捷键才配得上"更自由地使用"。`HotkeyRecorder` 已可复用，成本低 |
| 目标语言 | 窗口内「译为中文 / 译为英文」分段控件，落盘 `input_target` | 只说「输入翻译」不定义方向时，中文用户最常见的两个方向都要覆盖；现有管线（有道/必应/百度）都支持这两个方向，无需新源 |
| 有道在 `to="en"` 时 | 跳过并计入降级链错误，由必应接手 | 有道 `jsonapi` 只能英→中（词典 + 已下线的整句节点），不支持中→英 |
| 参数传递 | `translate` 命令加可选参 `to`，缺省 `"zh"` | 现有前端调用 `{ text }` 全部无需改动（`Option<String>` 可省略）；不新增命令 |
| 渲染复用 | 抽 `SenseList` 组件，Popup / ShotCard / InputPage 三处共用 | 释义列表渲染此前已经复制两份，本计划会引入第三份，顺手收敛，避免三套样式漂移 |
| 高度 | 复用计划①的 `useAutoFitWindow("input", 29)` | 带标题栏窗口由 `fit_window` 扣除标题栏高度，无需在设置窗口那套固定尺寸上再发明一种做法 |

## Global Constraints

- 不新增依赖（Cargo 与 npm 均无新增）
- 新配置字段必须 `#[serde(default = ...)]`，旧 `config.json` 能直接读起来（默认 `input_hotkey = "Ctrl+Alt+I"`、`input_target = "zh"`）
- 现有行为不变：`selection` 事件、`screenshot-result` 事件、`translate`/`get_config`/`save_config` 签名（`to` 为可选参，向后兼容）、主弹窗与截图窗口配置都不动
- 三个快捷键两两不同；校验放在后端 `save_config`，前端不再重复实现一套
- `input` 窗口不调用任何 core 窗口 API（只用自定义命令 `fit_window`），因此 `capabilities/default.json` 不需要改动
- 注释与用户可见文案一律中文；禁止 TODO/TBD
- Rust 任务收尾必须 `cd src-tauri && cargo test` 通过；前端任务收尾必须 `npx tsc --noEmit` 通过
- 提交信息中文，格式 `feat|fix|test|docs: 描述`

---

### Task 1: 翻译管线支持目标语言 `to`

**Files:**
- Modify: `src-tauri/src/translate.rs`
- Modify: `src-tauri/src/main.rs:401`（`screenshot_finish` 里的 `translate_text` 调用）
- Modify: `src/types.ts`（`AppConfig` 补两个新字段）
- Test: `src-tauri/src/translate.rs` 的 `mod tests`

**Interfaces:**
- Produces:
  - `AppConfig.input_hotkey: String`（默认 `"Ctrl+Alt+I"`）、`AppConfig.input_target: String`（默认 `"zh"`）
  - `fn bing_target(to: &str) -> &'static str`、`fn baidu_target(to: &str) -> &'static str`
  - `async fn try_translate(source: &str, text: &str, baidu: &BaiduConfig, to: &str) -> Result<WordInfo, String>`
  - `pub async fn translate_text(cfg: &AppConfig, text: &str, to: &str) -> Result<WordInfo, String>`（**签名变更**：加 `to`）
  - 命令 `translate(state, text: String, to: Option<String>)`（**签名变更**：加可选参）

- [ ] **Step 1: 写失败测试**

`translate.rs` 的 `mod tests` 追加：

```rust
    #[test]
    fn config_defaults_fill_input_fields() {
        // 旧配置文件没有这两个字段，serde 默认值必须兜底
        let cfg: AppConfig = serde_json::from_str("{}").unwrap();
        assert_eq!(cfg.input_hotkey, "Ctrl+Alt+I");
        assert_eq!(cfg.input_target, "zh");
    }

    #[test]
    fn targets_map_to_each_api() {
        assert_eq!(bing_target("zh"), "zh-Hans");
        assert_eq!(bing_target("en"), "en");
        assert_eq!(baidu_target("zh"), "zh");
        assert_eq!(baidu_target("en"), "en");
        // 非法值按中文处理
        assert_eq!(bing_target("fr"), "zh-Hans");
        assert_eq!(baidu_target("fr"), "zh");
    }

    #[tokio::test]
    async fn youdao_rejects_non_chinese_target() {
        let cfg = AppConfig::default();
        let err = try_translate("youdao", "hello", &cfg.baidu, "en")
            .await
            .expect_err("译为英文应拒绝有道");
        assert!(err.contains("仅支持译为中文"), "错误信息应说明原因: {err}");
    }

    #[tokio::test]
    async fn chinese_input_translates_to_english_via_bing() {
        // 未配置百度：中→英只能走必应（有道不支持该方向）
        let cfg = AppConfig::default();
        let info = translate_text(&cfg, "今天天气很好", "en")
            .await
            .expect("中译英应有必应结果");
        assert_eq!(info.source, "bing");
        assert!(!info.senses.is_empty());
        println!("en result: {:?}", info.senses);
    }
```

同时把现有两处调用改成三参数（行为不变，仍是译中）：

```rust
        let info = translate_text(&cfg, "The rain in Spain falls mainly on the plain")
```
→
```rust
        let info = translate_text(&cfg, "The rain in Spain falls mainly on the plain", "zh")
```
以及 `default_strategy_falls_back_to_bing_without_baidu_keys` 中的同名调用。

- [ ] **Step 2: 跑测试确认失败**

Run: `cd src-tauri && cargo test config_defaults_fill_input_fields targets_map_to_each_api`

Expected: 编译失败（`input_hotkey` 字段不存在 / `translate_text` 参数数量不匹配）

- [ ] **Step 3: 实现配置字段**

`translate.rs` 的 `AppConfig` 中，`ocr_lang` 字段之后追加：

```rust
    /// 输入翻译窗口的唤出快捷键，如 "Ctrl+Alt+I"
    #[serde(default = "default_input_hotkey")]
    pub input_hotkey: String,
    /// 输入翻译的目标语言："zh" | "en"
    #[serde(default = "default_input_target")]
    pub input_target: String,
```

默认值函数与 `impl Default for AppConfig` 同步补齐：

```rust
fn default_input_hotkey() -> String {
    "Ctrl+Alt+I".into()
}

fn default_input_target() -> String {
    "zh".into()
}
```

```rust
        AppConfig {
            source: default_source(),
            hotkey: default_hotkey(),
            screenshot_hotkey: default_screenshot_hotkey(),
            input_hotkey: default_input_hotkey(),
            input_target: default_input_target(),
            ocr_lang: default_ocr_lang(),
            baidu: BaiduConfig::default(),
        }
```

`src/types.ts` 的 `AppConfig` 接口同步补字段：

```ts
export interface AppConfig {
  source: string;
  hotkey: string;
  screenshot_hotkey: string;
  input_hotkey: string;
  input_target: string;
  ocr_lang: string;
  baidu: BaiduConfig;
}
```

- [ ] **Step 4: 实现目标语言贯通**

`translate_bing` 签名与目标语言改为：

```rust
/// 必应翻译（免 Key）：从 cn.bing.com 取临时凭证后调 ttranslatev3
async fn translate_bing(text: &str, to: &str) -> Result<WordInfo, String> {
    let t = bing_token().await?;
    let form = [
        ("fromLang", "auto-detect".to_string()),
        ("to", bing_target(to).to_string()),
        ("text", text.to_string()),
        ("token", t.token),
        ("key", t.key),
    ];
```

`translate_baidu` 签名与 `to` 参数改为：

```rust
async fn translate_baidu(text: &str, appid: &str, secret: &str, to: &str) -> Result<WordInfo, String> {
```

其 `.query(...)` 中 `("to", "zh")` 改为 `("to", baidu_target(to))`。

新增映射函数（放在 `try_translate` 上方）：

```rust
/// 目标语言 → 各接口的参数写法；非法值一律按中文处理
pub(crate) fn bing_target(to: &str) -> &'static str {
    if to == "en" { "en" } else { "zh-Hans" }
}

pub(crate) fn baidu_target(to: &str) -> &'static str {
    if to == "en" { "en" } else { "zh" }
}
```

`try_translate` 改为：

```rust
async fn try_translate(
    source: &str,
    text: &str,
    baidu: &BaiduConfig,
    to: &str,
) -> Result<WordInfo, String> {
    match source {
        "baidu" => translate_baidu(text, &baidu.appid, &baidu.secret, to).await,
        "bing" => translate_bing(text, to).await,
        // 有道 jsonapi 只能英→中（词典 + 已下线的整句节点），译为其它语言交给降级链
        _ if to == "zh" => translate_youdao(text).await,
        _ => Err("有道词典仅支持译为中文".into()),
    }
}
```

`translate_text` 改为：

```rust
/// 翻译核心：默认策略为单词走有道、整句回落必应；仅当配置了百度密钥才优先/降级走百度。
/// to: "zh" | "en"（其它值按 "zh" 处理）。
/// 从 translate 命令抽出，供截图翻译管线在无 State 的上下文直接调用。
pub async fn translate_text(cfg: &AppConfig, text: &str, to: &str) -> Result<WordInfo, String> {
    let to = if to == "en" { "en" } else { "zh" };
    let baidu_ready = !cfg.baidu.appid.is_empty() && !cfg.baidu.secret.is_empty();
    // 配置了百度 → 主源为百度（保留用户手动选择），否则默认有道开头
    let mut order: Vec<String> = vec![if baidu_ready {
        cfg.source.clone()
    } else {
        "youdao".into()
    }];
    // 降级顺序：有道 → 必应（免 Key 兜底）→ 百度（需已配置）
    for s in ["youdao", "bing", "baidu"] {
        if !order.contains(&s.to_string()) {
            order.push(s.to_string());
        }
    }

    let mut errors: Vec<String> = Vec::new();
    for src in order {
        if src == "baidu" && !baidu_ready {
            errors.push("[百度] 未配置 appid/secret".into());
            continue;
        }
        match try_translate(&src, text, &cfg.baidu, to).await {
            Ok(result) => return Ok(result),
            Err(e) => errors.push(format!("[{src}] {e}")),
        }
    }
    Err(errors.join("；"))
}

#[tauri::command]
pub async fn translate(
    state: State<'_, SharedConfig>,
    text: String,
    to: Option<String>,
) -> Result<WordInfo, String> {
    let cfg = state.lock().clone();
    translate_text(&cfg, &text, to.as_deref().unwrap_or("zh")).await
}
```

`main.rs` 的 `screenshot_finish` 里调用点改为：

```rust
                    match tauri::async_runtime::block_on(crate::translate::translate_text(&cfg, &text, "zh")) {
```

- [ ] **Step 5: 跑测试确认通过**

Run: `cd src-tauri && cargo test`

Expected: 全部 PASS（`chinese_input_translates_to_english_via_bing` 走真实网络，需联网）

- [ ] **Step 6: 提交**

```bash
git add src-tauri/src/translate.rs src-tauri/src/main.rs src/types.ts
git commit -m "feat: 翻译管线支持目标语言参数（译为中文/英文）"
```

---

### Task 2: 输入翻译窗口与页面

**Files:**
- Modify: `src-tauri/tauri.conf.json`（`windows` 数组新增 `input` 窗口）
- Modify: `src-tauri/src/main.rs`（托盘菜单项、`open_input_window`、`.card` 之外的 `✕ 只隐藏` 逻辑覆盖 `input`）
- Create: `src/components/InputPage.tsx`
- Create: `src/components/SenseList.tsx`
- Create: `src/lib/config.ts`
- Modify: `src/App.tsx`（新增 `?page=input` 路由）
- Modify: `src/components/Popup.tsx`、`src/components/ShotCard.tsx`（改用 `SenseList`）
- Modify: `src/components/SettingsPage.tsx`（删除本地 `patchConfig`，改用 `src/lib/config.ts`）
- Modify: `src/style.css`（输入窗口样式）

**Interfaces:**
- Consumes: Task 1 的 `translate({ text, to })`；计划①的 `useAutoFitWindow(label, chrome)`
- Produces:
  - 窗口 `label: "input"`，路由 `/?page=input`
  - `fn open_input_window(app: &AppHandle)`（托盘菜单与快捷键共用）
  - `patchConfig(patch: Partial<AppConfig>): Promise<void>`（`src/lib/config.ts`，设置页与输入页共用）
  - `<SenseList senses={string[]} className?: string />`

- [ ] **Step 1: 注册窗口**

`tauri.conf.json` 的 `app.windows` 中，`settings` 窗口之后插入：

```json
      {
        "label": "input",
        "title": "Wordpeek 输入翻译",
        "url": "/?page=input",
        "width": 460,
        "height": 240,
        "visible": false,
        "resizable": true,
        "decorations": true
      },
```

- [ ] **Step 2: 后端唤出逻辑与托盘菜单**

`main.rs` 中，`open_settings` 命令之后插入：

```rust
/// 显示输入翻译窗口（托盘菜单 / 输入翻译快捷键共用）
fn open_input_window(app: &AppHandle) {
    if let Some(win) = app.get_webview_window("input") {
        let _ = win.show();
        let _ = win.set_focus();
    }
}
```

`setup` 里把「设置窗口 ✕ 只隐藏」的逻辑扩展到 `input`（原 `if let Some(win) = app.get_webview_window("settings") { ... }` 整块替换）：

```rust
            // 设置/输入窗口点 ✕ 时只隐藏不销毁，否则窗口一旦关闭
            // 就无法再次打开（get_webview_window 返回 None）
            for label in ["settings", "input"] {
                if let Some(win) = app.get_webview_window(label) {
                    let w = win.clone();
                    win.on_window_event(move |event| {
                        if let WindowEvent::CloseRequested { api, .. } = event {
                            api.prevent_close();
                            let _ = w.hide();
                        }
                    });
                }
            }
```

托盘菜单加一项「输入翻译」：

```rust
            let input = MenuItem::with_id(app, "input", "输入翻译", true, None::<&str>)?;
            let settings =
                MenuItem::with_id(app, "settings", "设置", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
            let tray_menu = Menu::with_items(app, &[&input, &settings, &quit])?;
```

`on_menu_event` 的分支补上：

```rust
                .on_menu_event(|app, event| match event.id().as_ref() {
                    "input" => open_input_window(app),
                    "settings" => open_settings(app.clone()),
                    "quit" => app.exit(0),
                    _ => {}
                })
```

- [ ] **Step 3: 共享后排：`SenseList` 与 `patchConfig`**

创建 `src/components/SenseList.tsx`：

```tsx
import { splitSense } from "../lib/senses";

/** 释义列表：Popup / ShotCard / InputPage 三处共用，避免样式各写一套 */
export default function SenseList({
  senses,
  className = "",
}: {
  senses: string[];
  className?: string;
}) {
  return (
    <ul className={("senses " + className).trim()}>
      {senses.map((s, i) => {
        const { pos, text } = splitSense(s);
        return (
          <li key={i}>
            {pos && <b className="pos">{pos}</b>}
            {text}
          </li>
        );
      })}
    </ul>
  );
}
```

创建 `src/lib/config.ts`：

```ts
import { invoke } from "@tauri-apps/api/core";
import type { AppConfig } from "../types";

/** 以后端当前配置为基底，只改指定字段后整体写回——避免把表单里未保存的编辑一并落盘 */
export async function patchConfig(patch: Partial<AppConfig>) {
  const cfg = await invoke<AppConfig>("get_config");
  await invoke("save_config", { config: { ...cfg, ...patch } });
}
```

`SettingsPage.tsx`：删掉本地 `const patchConfig = async (patch: Partial<AppConfig>) => {...}` 定义，改为顶部 import：

```ts
import { patchConfig } from "../lib/config";
```

`Popup.tsx` 的释义块替换为 `<SenseList senses={info.senses} />`（原有 `splitSense` import 若不再使用则一并删除）；`ShotCard.tsx` 的释义块替换为 `<SenseList senses={shotResult.info.senses} className="shot-senses" />`。

- [ ] **Step 4: 路由与输入页**

`src/App.tsx`：

```tsx
import Popup from "./components/Popup";
import InputPage from "./components/InputPage";
import SettingsPage from "./components/SettingsPage";
import ScreenshotPage from "./components/ScreenshotPage";

export default function App() {
  // 设置/输入/截图窗口加载同一前端页面，用 URL 参数区分
  const page = new URLSearchParams(window.location.search).get("page");
  if (page === "settings") return <SettingsPage />;
  if (page === "input") return <InputPage />;
  if (page === "screenshot") return <ScreenshotPage />;
  return <Popup />;
}
```

创建 `src/components/InputPage.tsx`：

```tsx
import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { AppConfig, WordInfo } from "../types";
import { useAutoFitWindow } from "../lib/autofit";
import { patchConfig } from "../lib/config";
import SenseList from "./SenseList";
import { CheckIcon, CopyIcon } from "./icons";

/** 输入翻译窗口（?page=input）：打字/粘贴 → 译为中文或英文 */
export default function InputPage() {
  const [text, setText] = useState("");
  const [target, setTarget] = useState<"zh" | "en">("zh");
  const [info, setInfo] = useState<WordInfo | null>(null);
  const [error, setError] = useState("");
  const [loading, setLoading] = useState(false);
  const [copied, setCopied] = useState(false);
  // .input-layout 上下 padding 14×2 + 1px 余量
  const bodyRef = useAutoFitWindow("input", 29);
  const taRef = useRef<HTMLTextAreaElement | null>(null);

  useEffect(() => {
    invoke<AppConfig>("get_config")
      .then((cfg) => setTarget(cfg.input_target === "en" ? "en" : "zh"))
      .catch(() => {});
    // 窗口每次被唤到前台都聚焦输入框，省一次点击
    const focus = () => taRef.current?.focus();
    focus();
    window.addEventListener("focus", focus);
    return () => window.removeEventListener("focus", focus);
  }, []);

  const run = async () => {
    const q = text.trim();
    if (!q) return;
    setLoading(true);
    setError("");
    setInfo(null);
    try {
      // 翻译请求由 Rust 端发出（避免 CORS，且 API 密钥不进前端）
      setInfo(await invoke<WordInfo>("translate", { text: q, to: target }));
    } catch (err) {
      setError(String(err));
    } finally {
      setLoading(false);
    }
  };

  const changeTarget = (t: "zh" | "en") => {
    setTarget(t);
    patchConfig({ input_target: t }).catch(() => {});
  };

  const copyResult = () => {
    if (!info) return;
    invoke("copy_text", { text: info.senses.join("\n") })
      .then(() => {
        setCopied(true);
        setTimeout(() => setCopied(false), 1500);
      })
      .catch((e) => setError(String(e)));
  };

  return (
    <div className="input-layout">
      <div className="card-body" ref={bodyRef}>
        <textarea
          ref={taRef}
          className="input-text"
          value={text}
          rows={4}
          spellCheck={false}
          placeholder="输入或粘贴要翻译的文本，Ctrl+Enter 翻译"
          onChange={(e) => setText(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) {
              e.preventDefault();
              run();
            }
          }}
        />
        <div className="input-actions">
          <div className="segmented">
            <button
              className={target === "zh" ? "on" : ""}
              onClick={() => changeTarget("zh")}
            >
              译为中文
            </button>
            <button
              className={target === "en" ? "on" : ""}
              onClick={() => changeTarget("en")}
            >
              译为英文
            </button>
          </div>
          <button className="save-btn" onClick={run} disabled={loading || !text.trim()}>
            {loading ? "翻译中…" : "翻译"}
          </button>
        </div>
        {error && <div className="error">{error}</div>}
        {loading && <div className="loading">翻译中…</div>}
        {info && (
          <>
            <SenseList senses={info.senses} />
            <button
              className={"icon-btn" + (copied ? " ok" : "")}
              title="复制译文"
              onClick={copyResult}
            >
              {copied ? <CheckIcon /> : <CopyIcon />}
            </button>
          </>
        )}
      </div>
    </div>
  );
}
```

- [ ] **Step 5: 样式**

`src/style.css` 末尾追加：

```css
/* ===== 输入翻译窗口 ===== */
.input-layout {
  padding: 14px 16px;
  box-sizing: border-box;
  background: #ffffff;
  color: var(--text);
  font: 13.5px/1.65 -apple-system, "Segoe UI", "Microsoft YaHei", sans-serif;
}

textarea.input-text {
  width: 100%;
  box-sizing: border-box;
  padding: 8px 10px;
  border: 1px solid var(--border);
  border-radius: 6px;
  background: var(--bg-subtle);
  color: var(--text);
  font: inherit;
  resize: vertical;
}
textarea.input-text:focus {
  outline: none;
  box-shadow: inset 0 0 0 1.5px #4f8cff;
}

.input-actions {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-top: 10px;
  gap: 10px;
}

.segmented {
  display: inline-flex;
  border: 1px solid var(--border);
  border-radius: 6px;
  overflow: hidden;
}
.segmented button {
  border: none;
  background: #ffffff;
  color: var(--text-secondary);
  font: inherit;
  padding: 4px 10px;
  cursor: pointer;
}
.segmented button + button { border-left: 1px solid var(--border); }
.segmented button.on {
  background: var(--bg-subtle);
  color: var(--text);
  font-weight: 600;
}
```

- [ ] **Step 6: 类型检查 + 实机验证**

Run: `npx tsc --noEmit`

Expected: 无输出

Run: `npm run tauri dev`，逐条验证：

1. 托盘菜单 → 「输入翻译」→ 窗口出现，输入框已聚焦
2. 粘贴 `The quick brown fox jumps over the lazy dog` → 点「翻译」→ 中文译文；`Ctrl+Enter` 同样触发
3. 切到「译为英文」，粘贴 `今天天气很好` → 英文译文（必应）
4. 关掉输入窗口（点 ✕）→ 再点托盘「输入翻译」→ 窗口重新出现，之前输入仍在
5. 空文本时「翻译」按钮禁用；翻译失败（拔网线）→ 显示可读错误
6. 窗口高度：内容变化时贴合内容，不超出屏幕工作区 70%；手动拉宽后宽度保持
7. 划词弹窗与截图卡片外观不变（`SenseList` 重构未改样式）

- [ ] **Step 7: 提交**

```bash
git add src-tauri/tauri.conf.json src-tauri/src/main.rs src/App.tsx \
  src/components/InputPage.tsx src/components/SenseList.tsx src/lib/config.ts \
  src/components/Popup.tsx src/components/ShotCard.tsx src/components/SettingsPage.tsx src/style.css
git commit -m "feat: 新增输入翻译窗口（托盘菜单唤出，支持译中/译英）"
```

---

### Task 3: 输入翻译快捷键 + 三键冲突校验 + README

**Files:**
- Modify: `src-tauri/src/main.rs`（`register_hotkeys` 加第三键；新增 `validate_hotkeys` 纯函数）
- Modify: `src-tauri/src/translate.rs`（`save_config` 先校验快捷键）
- Modify: `src/components/SettingsPage.tsx`（快捷键分组加第三个录制框；删除前端重复的「两键不能相同」检查）
- Modify: `README.md`

**Interfaces:**
- Produces:
  - `pub(crate) fn validate_hotkeys(hotkeys: &[&str]) -> Result<(), String>`
  - `register_hotkeys` 注册三个快捷键；`save_config` 在写入前用 `validate_hotkeys` 拒绝重复

- [ ] **Step 1: 写失败测试**

`main.rs` 的 `mod tests` 追加：

```rust
    #[test]
    fn validate_hotkeys_rejects_duplicates() {
        assert!(validate_hotkeys(&["Ctrl+Alt+T", "Ctrl+Alt+S", "Ctrl+Alt+I"]).is_ok());
        // 大小写/空白不同视为同一个键
        assert!(validate_hotkeys(&["Ctrl+Alt+T", "ctrl+alt+t", "Ctrl+Alt+I"]).is_err());
        assert!(validate_hotkeys(&["Ctrl+Alt+T", "Ctrl+Alt+S", "Ctrl+Alt+S"]).is_err());
        let err = validate_hotkeys(&["Ctrl+Alt+T", "Ctrl+Alt+T", "Ctrl+Alt+I"]).unwrap_err();
        assert!(err.contains("重复"), "错误信息应说明原因: {err}");
    }
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cd src-tauri && cargo test validate_hotkeys_rejects_duplicates`

Expected: 编译失败，`cannot find function validate_hotkeys`

- [ ] **Step 3: 实现校验与第三键注册**

`main.rs` 中，`register_hotkeys` 上方插入：

```rust
/// 快捷键两两不能重复（重复会导致行为歧义）；大小写与首尾空白不同视为同一个键
pub(crate) fn validate_hotkeys(hotkeys: &[&str]) -> Result<(), String> {
    let mut seen: Vec<String> = Vec::new();
    for h in hotkeys {
        let key = h.trim().to_lowercase();
        if seen.contains(&key) {
            return Err(format!("快捷键重复：{h}"));
        }
        seen.push(key);
    }
    Ok(())
}
```

`register_hotkeys` 的重复检查改为调用它，并追加第三键注册（原「两个快捷键相同」分支整段替换）：

```rust
pub(crate) fn register_hotkeys(app: &AppHandle, cfg: &translate::AppConfig) {
    if let Err(e) = app.global_shortcut().unregister_all() {
        eprintln!("[shortcut] 注销旧快捷键失败: {e}");
    }
    // 任意两个快捷键相同会导致行为歧义，直接都不注册
    if let Err(e) = validate_hotkeys(&[&cfg.hotkey, &cfg.screenshot_hotkey, &cfg.input_hotkey]) {
        eprintln!("[shortcut] {e}");
        return;
    }
    if let Ok(sc) = parse_hotkey(&cfg.hotkey) {
        if let Err(e) = app.global_shortcut().on_shortcut(sc, |app, _s, event| {
            if event.state == ShortcutState::Pressed {
                handle_hotkey_trigger(app);
            }
        }) {
            eprintln!("[shortcut] {} 注册失败: {e}", cfg.hotkey);
        }
    }
    if let Ok(sc) = parse_hotkey(&cfg.screenshot_hotkey) {
        if let Err(e) = app.global_shortcut().on_shortcut(sc, |app, _s, event| {
            if event.state == ShortcutState::Pressed {
                handle_screenshot_trigger(app);
            }
        }) {
            eprintln!("[shortcut] {} 注册失败: {e}", cfg.screenshot_hotkey);
        }
    }
    if let Ok(sc) = parse_hotkey(&cfg.input_hotkey) {
        if let Err(e) = app.global_shortcut().on_shortcut(sc, |app, _s, event| {
            if event.state == ShortcutState::Pressed {
                open_input_window(app);
            }
        }) {
            eprintln!("[shortcut] {} 注册失败: {e}", cfg.input_hotkey);
        }
    }
}
```

`translate.rs` 的 `save_config` 在写盘前加校验（放在 `config_path` 之前）：

```rust
    crate::validate_hotkeys(&[&config.hotkey, &config.screenshot_hotkey, &config.input_hotkey])?;
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cd src-tauri && cargo test`

Expected: 全部 PASS

- [ ] **Step 5: 设置页加第三个录制框**

`SettingsPage.tsx`：

1. 状态追加 `const [inputHotkey, setInputHotkey] = useState("Ctrl+Alt+I");`
2. 读配置的 effect 里追加 `setInputHotkey(cfg.input_hotkey || "Ctrl+Alt+I");`
3. `save()` 里删掉这段前端重复校验（后端 `save_config` 已返回同样的错误）：

```tsx
      if (hotkey.trim().toLowerCase() === screenshotHotkey.trim().toLowerCase()) {
        setSaveError("划词与截图快捷键不能相同");
        return;
      }
```

4. `save()` 提交的 config 里补两个字段：

```tsx
        config: {
          source,
          hotkey,
          screenshot_hotkey: screenshotHotkey,
          input_hotkey: inputHotkey,
          input_target: target,
          ocr_lang: ocrLang,
          baidu: { appid: appid.trim(), secret: secret.trim() },
        },
```

其中 `target` 取当前输入翻译目标语言——设置页不展示该字段（枚举值由输入窗口内的分段控件维护），因此改为从后端读回后原样带回：在 `save()` 里改为

```tsx
      const current = await invoke<AppConfig>("get_config");
      await invoke("save_config", {
        config: {
          ...current,
          source,
          hotkey,
          screenshot_hotkey: screenshotHotkey,
          input_hotkey: inputHotkey,
          ocr_lang: ocrLang,
          baidu: { appid: appid.trim(), secret: secret.trim() },
        },
      });
```

5. 「快捷键」分组追加录制框与提示：

```tsx
            <label className="field">
              输入翻译快捷键
              <HotkeyRecorder value={inputHotkey} onChange={setInputHotkey} />
            </label>
            <div className="field-hint">点击输入框后按下组合键即可更换，需包含 Ctrl / Alt / Shift 之一；三个快捷键不能相同</div>
```

- [ ] **Step 6: README 更新**

`README.md`：功能列表加「**输入翻译**：`Ctrl+Alt+I`（或托盘菜单）唤出窗口，打字/粘贴即译，支持译为中文或英文」；「使用说明」加一条「托盘菜单 → 输入翻译，或按 `Ctrl+Alt+I`，输入文本后 `Ctrl+Enter` 翻译」；快捷键表加一行 `| 输入翻译 | Ctrl+Alt+I |`；目录结构里把 `src/components/InputPage.tsx` 一并列上。

- [ ] **Step 7: 类型检查 + 实机验证**

Run: `npx tsc --noEmit` → 无输出

Run: `npm run tauri dev`：

1. 按 `Ctrl+Alt+I` → 输入窗口出现并聚焦
2. 设置里把「输入翻译快捷键」改成 `Ctrl+Alt+U` 保存 → 旧键失效、新键生效
3. 把「输入翻译快捷键」与「划词快捷键」录成同一个键保存 → 设置页报「快捷键重复：…」，配置未写入（重启后旧值仍在）
4. 输入翻译窗口切「译为英文」后重启应用 → 目标语言记忆为英文

- [ ] **Step 8: 提交**

```bash
git add src-tauri/src/main.rs src-tauri/src/translate.rs src/components/SettingsPage.tsx README.md
git commit -m "feat: 输入翻译快捷键 Ctrl+Alt+I 与三键重复校验"
```

---

## 验收清单（对应 issue #2 第 1 条）

- [ ] 托盘菜单「输入翻译」与 `Ctrl+Alt+I` 都能唤出窗口
- [ ] 输入/粘贴文本 → 译为中文；切「译为英文」→ 中译英走必应
- [ ] 空文本不触发请求；失败有可读错误
- [ ] 目标语言、快捷键都持久化在 `config.json`；三个快捷键重复时保存被拒
- [ ] 划词弹窗、截图卡片行为与外观不变（`SenseList` 重构零变化）
- [ ] `cargo test`、`npx tsc --noEmit` 通过

## 非目标（本期不做）

- 输入翻译窗口内不接「多源对比」（由计划 `2026-10-13-multi-source-compare.md` 接入同一窗口）
- 不做自动翻译（输入即译）与历史记录
- 不做源语言手动指定：沿用各接口的 `auto-detect`
