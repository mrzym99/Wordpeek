# 多翻译源对比 实施计划（issue #2 第 4 条）

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 同一段文本一次看到有道/必应/百度各自的译文，逐行复制或一键「固定为默认源」——哪家好用用哪家。

**Architecture:** 后端新增 `translate_all` 命令：并发把文本发给所有可用源（每个源一个 `tauri::async_runtime::spawn` 任务，`reqwest` 请求天然并行），逐源返回成功译文或错误，**整体永不失败**（避免一个源挂掉就没有对比页）。前端新增 `SourceCompare` 组件渲染逐源行（含复制、固定），接入输入翻译窗口（计划②）与划词弹窗；降级顺序抽成纯函数 `build_order` 并新增 `preferred` 配置项支撑「固定为默认源」。

**Tech Stack:** Tauri 2（`tauri::async_runtime::{spawn, JoinHandle}`，已核实返回 `tauri::Result<T>`）、reqwest 0.12、React 18。

**Spec:** `https://github.com/mrzym99/Wordpeek/issues/2` 第 4 条；依赖 `2026-10-13-input-translate.md`（输入窗口）与 `2026-10-13-popup-autosize.md`（卡片自适应高度）。

## 设计决策

| 决策 | 选择 | 理由 / 否决的替代方案 |
|---|---|---|
| 触发方式 | 用户显式点「对比各源」（输入窗口的开关 / 卡片上的按钮），不默认开启 | 默认全源并发会让每次划词都发 2～3 个请求：延迟和流量翻倍，而多数时候用户只想快速看一个结果 |
| 后端模型 | 新命令 `translate_all` 返回 `Vec<SourceResult>`，逐行 `ok/error` | 复用 `translate` 无法表达「部分失败」；对比页必须能展示「有道这句没词条」而不是整页报错 |
| 并发方式 | 每源一个 `tauri::async_runtime::spawn` 任务，按固定顺序收集 | 不引 `futures` 依赖；`tokio::join!` 在源数量可变（2～3 个，看百度是否配置）时写起来很别扭 |
| 「用那个」 | 逐行复制 + 「固定为默认源」写入新配置 `preferred` | 复制只解决一次；用户说"哪个好用用哪个"意味着后续查询也该走它。固定源失败时仍按 有道→必应→百度 降级，不会把自己锁死 |
| `preferred` 与既有 `source` | 并存：`preferred` 是用户显式固定（优先级最高），`source` 仍是「百度作主源」的开关 | 现有 `source` 只在配置了百度密钥时才生效；直接改 `source` 表达不了"固定必应" |
| 对比页位置 | 输入翻译窗口 + 划词弹窗；截图卡片不做 | 截图卡片已经有 OCR 原文编辑区，再塞多源结果会挤；用户可复制原文到输入窗口对比 |
| 对比开关是否持久化 | 不持久化（每次打开默认关闭） | 纯展示偏好，避免配置项继续膨胀；真正需要持久化的是 `preferred` |

## Global Constraints

- 不新增依赖（Cargo 与 npm 均无新增）
- 新配置字段必须 `#[serde(default)]`：`preferred: String`（默认 `""` = 自动）
- 现有行为不变：`translate` 命令签名与语义、降级链默认顺序（未配百度 → 有道→必应）、`selection`/`screenshot-result` 事件、弹窗自动隐藏逻辑
- 全部失败时的错误文案保持现状：仍包含 `[百度] 未配置 appid/secret` 提示（未配置密钥时）
- 注释与用户可见文案一律中文；禁止 TODO/TBD
- Rust 任务收尾必须 `cd src-tauri && cargo test` 通过；前端任务收尾必须 `npx tsc --noEmit` 通过
- 提交信息中文，格式 `feat|fix|test|docs: 描述`

---

### Task 1: 降级顺序抽成纯函数 + `preferred` 配置

**Files:**
- Modify: `src-tauri/src/translate.rs`
- Modify: `src/types.ts`（`AppConfig.preferred`）
- Test: `src-tauri/src/translate.rs` 的 `mod tests`

**Interfaces:**
- Produces:
  - `AppConfig.preferred: String`（`""` = 自动）
  - `fn push_source(order: &mut Vec<String>, source: &str, baidu_ready: bool, to: &str)`
  - `pub(crate) fn build_order(cfg: &AppConfig, to: &str) -> Vec<String>`
  - `translate_text(cfg, text, to)` 内部改为消费 `build_order`

- [ ] **Step 1: 写失败测试**

`translate.rs` 的 `mod tests` 追加：

```rust
    #[test]
    fn build_order_default_is_youdao_then_bing() {
        let cfg = AppConfig::default();
        assert_eq!(build_order(&cfg, "zh"), vec!["youdao", "bing"]);
        // 译为英文不进有道（它只支持英→中）
        assert_eq!(build_order(&cfg, "en"), vec!["bing"]);
    }

    #[test]
    fn build_order_puts_preferred_first() {
        let cfg = AppConfig {
            preferred: "bing".into(),
            ..AppConfig::default()
        };
        assert_eq!(build_order(&cfg, "zh"), vec!["bing", "youdao"]);
    }

    #[test]
    fn build_order_includes_baidu_only_when_configured() {
        let mut cfg = AppConfig::default();
        assert!(!build_order(&cfg, "zh").iter().any(|s| s == "baidu"));
        // 配了密钥但没把百度设为主源：仍是默认的主源（有道）在前，百度兜底
        cfg.baidu = BaiduConfig {
            appid: "a".into(),
            secret: "b".into(),
        };
        assert_eq!(build_order(&cfg, "zh"), vec!["youdao", "bing", "baidu"]);
        // 用户把百度设为主源：百度排第一
        cfg.source = "baidu".into();
        assert_eq!(build_order(&cfg, "zh"), vec!["baidu", "youdao", "bing"]);
        // 固定百度但没配密钥 → 不进队列
        cfg.baidu = BaiduConfig::default();
        cfg.preferred = "baidu".into();
        assert_eq!(build_order(&cfg, "zh"), vec!["youdao", "bing"]);
    }
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cd src-tauri && cargo test build_order`

Expected: 编译失败（`preferred` 字段与 `build_order` 都不存在）

- [ ] **Step 3: 实现**

`AppConfig` 的 `input_target` 之后追加：

```rust
    /// 用户固定的首选翻译源：""（自动） | "youdao" | "bing" | "baidu"
    #[serde(default)]
    pub preferred: String,
```

`impl Default for AppConfig` 里补 `preferred: String::new(),`。

`src/types.ts` 的 `AppConfig` 补 `preferred: string;`。

`try_translate` 上方插入：

```rust
/// 把一个源按可用性追加进降级队列（去重）。有道只参与译为中文；百度需已配置密钥
fn push_source(order: &mut Vec<String>, source: &str, baidu_ready: bool, to: &str) {
    let usable = match source {
        "baidu" => baidu_ready,
        "youdao" => to == "zh",
        "bing" => true,
        _ => false,
    };
    if usable && !order.iter().any(|s| s == source) {
        order.push(source.to_string());
    }
}

/// 降级顺序：用户固定的首选源 → 默认主源（配了百度密钥则用 source，否则有道）
/// → 有道 → 必应 → 百度。纯函数，便于断言顺序而不发网络请求。
pub(crate) fn build_order(cfg: &AppConfig, to: &str) -> Vec<String> {
    let baidu_ready = !cfg.baidu.appid.is_empty() && !cfg.baidu.secret.is_empty();
    let mut order: Vec<String> = Vec::new();
    if !cfg.preferred.is_empty() {
        push_source(&mut order, &cfg.preferred, baidu_ready, to);
    }
    if baidu_ready {
        push_source(&mut order, &cfg.source, baidu_ready, to);
    }
    for s in ["youdao", "bing", "baidu"] {
        push_source(&mut order, s, baidu_ready, to);
    }
    order
}
```

`translate_text` 函数体（归一化 `to` 之后的部分）替换为：

```rust
pub async fn translate_text(cfg: &AppConfig, text: &str, to: &str) -> Result<WordInfo, String> {
    let to = if to == "en" { "en" } else { "zh" };
    let baidu_ready = !cfg.baidu.appid.is_empty() && !cfg.baidu.secret.is_empty();
    let mut errors: Vec<String> = Vec::new();
    for src in build_order(cfg, to) {
        match try_translate(&src, text, &cfg.baidu, to).await {
            Ok(result) => return Ok(result),
            Err(e) => errors.push(format!("[{src}] {e}")),
        }
    }
    // 全部失败时补一句「百度还没配」，保留原有提示，方便用户知道还有可选源
    if !baidu_ready {
        errors.push("[百度] 未配置 appid/secret".into());
    }
    Err(errors.join("；"))
}
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cd src-tauri && cargo test`

Expected: 全部 PASS（含既有 `default_strategy_falls_back_to_bing_without_baidu_keys`，证明默认行为未变）

- [ ] **Step 5: 提交**

```bash
git add src-tauri/src/translate.rs src/types.ts
git commit -m "refactor: 降级顺序抽成 build_order 纯函数并新增 preferred 首选源配置"
```

---

### Task 2: `translate_all` 并发对比命令

**Files:**
- Modify: `src-tauri/src/translate.rs`
- Modify: `src-tauri/src/main.rs`（`invoke_handler` 注册 `translate_all`）
- Test: `src-tauri/src/translate.rs` 的 `mod tests`

**Interfaces:**
- Consumes: Task 1 的 `build_order` 之外的 `try_translate(source, text, baidu, to)`（Task 1 之前的 translate 计划中已定型）
- Produces:
  - `pub struct SourceResult { source: String, label: String, ok: bool, senses: Vec<String>, error: Option<String> }`
  - `pub(crate) fn source_label(source: &str) -> &'static str`
  - `pub(crate) fn compare_sources(cfg: &AppConfig, to: &str) -> Vec<&'static str>`
  - `pub async fn translate_all_sources(cfg: &AppConfig, text: &str, to: &str) -> Vec<SourceResult>`
  - 命令 `translate_all(state, text: String, to: Option<String>) -> Vec<SourceResult>`

- [ ] **Step 1: 写失败测试**

`translate.rs` 的 `mod tests` 追加：

```rust
    #[test]
    fn compare_sources_lists_all_usable() {
        let mut cfg = AppConfig::default();
        assert_eq!(compare_sources(&cfg, "zh"), vec!["youdao", "bing"]);
        assert_eq!(compare_sources(&cfg, "en"), vec!["bing"]);
        cfg.baidu = BaiduConfig {
            appid: "a".into(),
            secret: "b".into(),
        };
        assert_eq!(compare_sources(&cfg, "zh"), vec!["youdao", "bing", "baidu"]);
    }

    #[test]
    fn source_labels_are_human_readable() {
        assert_eq!(source_label("youdao"), "有道词典");
        assert_eq!(source_label("bing"), "必应翻译");
        assert_eq!(source_label("baidu"), "百度翻译");
        assert_eq!(source_label("nope"), "未知源");
    }

    #[tokio::test]
    async fn compare_reports_each_source_separately() {
        // 整句：有道没有词条（错误行），必应出译文（成功行）——整体不失败
        let cfg = AppConfig::default();
        let rows = translate_all_sources(
            &cfg,
            "The quick brown fox jumps over the lazy dog",
            "zh",
        )
        .await;
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].source, "youdao");
        assert!(!rows[0].ok);
        assert!(rows[0].error.is_some());
        assert_eq!(rows[1].source, "bing");
        assert!(rows[1].ok);
        assert!(rows[1].error.is_none());
        assert!(!rows[1].senses.is_empty());
        println!("compare rows: {rows:?}");
    }
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cd src-tauri && cargo test compare_

Expected: 编译失败（`compare_sources` / `translate_all_sources` 不存在）

- [ ] **Step 3: 实现**

`translate.rs` 的 `translate_text` 之后插入：

```rust
/// 单源对比结果：失败的源也返回，由前端逐行展示错误
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SourceResult {
    pub source: String,
    pub label: String,
    pub ok: bool,
    pub senses: Vec<String>,
    pub error: Option<String>,
}

/// 源展示名（对比行标题）
pub(crate) fn source_label(source: &str) -> &'static str {
    match source {
        "youdao" => "有道词典",
        "bing" => "必应翻译",
        "baidu" => "百度翻译",
        _ => "未知源",
    }
}

/// 参与对比的源：必应必在；有道的词典只对中文目标有意义；百度需已配置密钥。
/// 顺序固定，前端可稳定渲染对比行。
pub(crate) fn compare_sources(cfg: &AppConfig, to: &str) -> Vec<&'static str> {
    let baidu_ready = !cfg.baidu.appid.is_empty() && !cfg.baidu.secret.is_empty();
    let mut out: Vec<&'static str> = Vec::new();
    if to == "zh" {
        out.push("youdao");
    }
    out.push("bing");
    if baidu_ready {
        out.push("baidu");
    }
    out
}

/// 并发请求全部可用源并逐源收集结果：整体不失败，单个源出错只影响它自己那一行
pub async fn translate_all_sources(cfg: &AppConfig, text: &str, to: &str) -> Vec<SourceResult> {
    let to = if to == "en" { "en" } else { "zh" };
    // 每源一个异步任务：reqwest 请求并行发出，总耗时 ≈ 最慢的源
    let handles: Vec<(String, tauri::async_runtime::JoinHandle<Result<WordInfo, String>>)> =
        compare_sources(cfg, to)
            .into_iter()
            .map(|source| {
                let src = source.to_string();
                let query = text.to_string();
                let baidu = cfg.baidu.clone();
                let to = to.to_string();
                let handle = tauri::async_runtime::spawn(async move {
                    try_translate(&src, &query, &baidu, &to).await
                });
                (src, handle)
            })
            .collect();

    let mut out = Vec::with_capacity(handles.len());
    for (source, handle) in handles {
        // 任务 panic / 被取消也算该源失败，不影响其它行
        let result = match handle.await {
            Ok(r) => r,
            Err(e) => Err(format!("请求异常: {e}")),
        };
        out.push(match result {
            Ok(info) => SourceResult {
                label: source_label(&source).to_string(),
                source,
                ok: true,
                senses: info.senses,
                error: None,
            },
            Err(e) => SourceResult {
                label: source_label(&source).to_string(),
                source,
                ok: false,
                senses: Vec::new(),
                error: Some(e),
            },
        });
    }
    out
}

/// 对比命令：一次列出各源译文（输入窗口「对比各源」开关 / 卡片「对比各源」按钮）
#[tauri::command]
pub async fn translate_all(
    state: State<'_, SharedConfig>,
    text: String,
    to: Option<String>,
) -> Vec<SourceResult> {
    let cfg = state.lock().clone();
    translate_all_sources(&cfg, &text, to.as_deref().unwrap_or("zh")).await
}
```

`main.rs` 的 `invoke_handler` 里，`translate::translate,` 之后追加：

```rust
            translate::translate,
            translate::translate_all,
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cd src-tauri && cargo test`

Expected: 全部 PASS（`compare_reports_each_source_separately` 走真实网络）

- [ ] **Step 5: 提交**

```bash
git add src-tauri/src/translate.rs src-tauri/src/main.rs
git commit -m "feat: 新增 translate_all 并发多源对比命令"
```

---

### Task 3: 对比 UI（逐源行 + 复制 + 固定首选源）

**Files:**
- Create: `src/components/SourceCompare.tsx`
- Modify: `src/types.ts`（`SourceResult`）
- Modify: `src/components/InputPage.tsx`（「对比各源」开关）
- Modify: `src/components/Popup.tsx`（「对比各源」/「返回词典」按钮）
- Modify: `src/components/SettingsPage.tsx`（首选源下拉）
- Modify: `src/style.css`（对比行样式）

**Interfaces:**
- Consumes: `translate_all({ text, to }) -> SourceResult[]`（Task 2）、`patchConfig`（`src/lib/config.ts`）
- Produces: `<SourceCompare rows pinned copied onCopy onPin />`
  - `rows: SourceResult[]`、`pinned: string`、`copied: string`
  - `onCopy: (source: string, text: string) => void`、`onPin: (source: string) => void`

- [ ] **Step 1: 类型与组件**

`src/types.ts` 追加：

```ts
/** 逐源对比结果（translate_all 命令） */
export interface SourceResult {
  source: string;
  label: string;
  ok: boolean;
  senses: string[];
  error: string | null;
}
```

创建 `src/components/SourceCompare.tsx`：

```tsx
import type { SourceResult } from "../types";
import SenseList from "./SenseList";
import { CheckIcon, CopyIcon } from "./icons";

interface SourceCompareProps {
  rows: SourceResult[];
  /** 当前固定的首选源（"" = 自动） */
  pinned: string;
  /** 复制反馈标记：命中的行显示对勾 */
  copied: string;
  onCopy: (source: string, text: string) => void;
  onPin: (source: string) => void;
}

/** 多源对比列表：每源一行，含译文、复制、固定为默认源 */
export default function SourceCompare({ rows, pinned, copied, onCopy, onPin }: SourceCompareProps) {
  return (
    <div className="compare">
      {rows.map((r) => (
        <div className="compare-row" key={r.source}>
          <div className="compare-head">
            <span className="compare-label">{r.label}</span>
            <span className="compare-actions">
              <button
                className={"icon-btn" + (copied === r.source ? " ok" : "")}
                title="复制该源译文"
                disabled={!r.ok}
                onClick={() => onCopy(r.source, r.senses.join("\n"))}
              >
                {copied === r.source ? <CheckIcon /> : <CopyIcon />}
              </button>
              <button
                className={"text-btn" + (pinned === r.source ? " on" : "")}
                title="之后优先使用该源（失败仍会自动降级）"
                onClick={() => onPin(r.source)}
              >
                {pinned === r.source ? "已固定" : "固定为默认源"}
              </button>
            </span>
          </div>
          {r.ok ? <SenseList senses={r.senses} /> : <div className="compare-error">{r.error}</div>}
        </div>
      ))}
    </div>
  );
}
```

- [ ] **Step 2: 输入翻译窗口接入开关**

`InputPage.tsx` 改动：

1. import 追加 `import SourceCompare from "./SourceCompare";`、`import type { SourceResult } from "../types";`
2. 状态追加：

```tsx
  const [compare, setCompare] = useState(false);
  const [rows, setRows] = useState<SourceResult[] | null>(null);
  const [pinned, setPinned] = useState("");
  const [copiedKey, setCopiedKey] = useState("");
```

3. 读配置的 effect 里补 `setPinned(cfg.preferred || "");`
4. `run()` 改为：

```tsx
  const run = async () => {
    const q = text.trim();
    if (!q) return;
    setLoading(true);
    setError("");
    setInfo(null);
    setRows(null);
    try {
      if (compare) {
        setRows(await invoke<SourceResult[]>("translate_all", { text: q, to: target }));
      } else {
        setInfo(await invoke<WordInfo>("translate", { text: q, to: target }));
      }
    } catch (err) {
      setError(String(err));
    } finally {
      setLoading(false);
    }
  };

  const pin = (source: string) => {
    patchConfig({ preferred: source })
      .then(() => setPinned(source))
      .catch((e) => setError(String(e)));
  };

  const copyRow = (source: string, text: string) => {
    invoke("copy_text", { text })
      .then(() => {
        setCopiedKey(source);
        setTimeout(() => setCopiedKey(""), 1500);
      })
      .catch((e) => setError(String(e)));
  };
```

5. `.input-actions` 里加开关（放在分段控件左侧）：

```tsx
          <label className="switch">
            <input
              type="checkbox"
              checked={compare}
              onChange={(e) => {
                setCompare(e.target.checked);
                setRows(null);
                setInfo(null);
              }}
            />
            对比各源
          </label>
```

6. 结果区改为：

```tsx
        {loading && <div className="loading">翻译中…</div>}
        {rows && (
          <SourceCompare
            rows={rows}
            pinned={pinned}
            copied={copiedKey}
            onCopy={copyRow}
            onPin={pin}
          />
        )}
        {!rows && info && (
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
```

- [ ] **Step 3: 划词弹窗接入按钮**

`Popup.tsx` 改动：

1. import 追加 `import SourceCompare from "./SourceCompare";`、`import type { AppConfig, Selection, ShotResult, SourceResult, WordInfo } from "../types";`、`import { patchConfig } from "../lib/config";`
2. `const [copied, setCopied] = useState("");` 的类型放宽为 `useState<string>("")`（对比行按源名做标记），`doCopy` 的 `label` 参数类型由 `"text" | "translation"` 改为 `string`。
   `ShotCard` 不用改：它的 `onCopy` 形参类型更窄（`"text" | "translation"`），传一个形参类型更宽的 `doCopy` 进去是合法的（形参逆变）
3. 状态追加：

```tsx
  // 对比模式：非空时用多源结果替换词典卡片
  const [rows, setRows] = useState<SourceResult[] | null>(null);
  const [compareLoading, setCompareLoading] = useState(false);
  const [pinned, setPinned] = useState("");
```

4. `selection` 事件处理里，`setShotResult(null);` 之后追加 `setRows(null);`（新划词退出对比视图）
5. 新增：

```tsx
  const runCompare = async (text: string) => {
    setCompareLoading(true);
    setRows(null);
    setError("");
    try {
      setRows(await invoke<SourceResult[]>("translate_all", { text, to: "zh" }));
      setPinned((await invoke<AppConfig>("get_config")).preferred || "");
    } catch (err) {
      setError(String(err));
    } finally {
      setCompareLoading(false);
    }
  };

  const pin = (source: string) => {
    patchConfig({ preferred: source })
      .then(() => setPinned(source))
      .catch((e) => setError(String(e)));
  };
```

6. `.word-head` 里，齿轮按钮之前加：

```tsx
        {word && (
          <button
            className="text-btn"
            title={rows ? "返回词典卡片" : "同时查看各翻译源结果"}
            onClick={() => (rows ? setRows(null) : runCompare(word))}
          >
            {rows ? "返回词典" : "对比各源"}
          </button>
        )}
```

7. 正文区（`{error && ...}` 到 `{info && (...)}` 结束）替换为：

```tsx
      {error && <div className="error">{error}</div>}
      {!info && !error && !rows && !compareLoading && <div className="loading">翻译中...</div>}
      {compareLoading && <div className="loading">对比中…</div>}

      {rows && (
        <SourceCompare
          rows={rows}
          pinned={pinned}
          copied={copied}
          onCopy={doCopy}
          onPin={pin}
        />
      )}

      {!rows && info && (
        <>
          {/* ……原有 senses / word_forms / synos 渲染保持不变…… */}
        </>
      )}
```

- [ ] **Step 4: 设置页首选源**

`SettingsPage.tsx` 改动：

1. 状态追加 `const [preferred, setPreferred] = useState("");`
2. 读配置 effect 里补 `setPreferred(cfg.preferred || "");`
3. 新增：

```tsx
  // 首选源：固定后仍保留自动降级（失败会继续试其它源）
  const changePreferred = (v: string) => {
    setPreferred(v);
    patchConfig({ preferred: v }).catch((e) => setSaveError(String(e)));
  };
```

4. 「翻译」分组里，「百度翻译（可选）」分组标题之前插入：

```tsx
            <label className="field">
              首选源
              <select value={preferred} onChange={(e) => changePreferred(e.target.value)}>
                <option value="">自动（单词走有道，整句走必应）</option>
                <option value="youdao">有道词典</option>
                <option value="bing">必应翻译</option>
                <option value="baidu">百度翻译（需配置密钥）</option>
              </select>
            </label>
            <div className="field-hint">首选源失败时仍按 有道 → 必应 → 百度 自动降级</div>
```

- [ ] **Step 5: 样式**

`src/style.css` 末尾追加：

```css
/* ===== 多源对比 ===== */
.compare { margin-top: 10px; }
.compare-row + .compare-row {
  margin-top: 10px;
  padding-top: 8px;
  border-top: 1px solid var(--border);
}
.compare-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
}
.compare-label {
  font-size: 12px;
  font-weight: 600;
  color: var(--text-secondary);
}
.compare-actions {
  display: inline-flex;
  align-items: center;
  gap: 6px;
}
.compare-row .senses { margin-top: 2px; }
.compare-error {
  margin-top: 4px;
  color: var(--error);
  font-size: 12.5px;
  white-space: pre-wrap;
}

/* 文字按钮：卡片/设置页里的小动作（对比各源 / 固定为默认源） */
.text-btn {
  border: 1px solid var(--border);
  border-radius: 4px;
  background: #ffffff;
  color: var(--text-secondary);
  font: inherit;
  font-size: 12px;
  padding: 1px 8px;
  cursor: pointer;
  white-space: nowrap;
}
.text-btn:hover { border-color: var(--accent); color: var(--accent); }
.text-btn.on { border-color: var(--accent); color: var(--accent); }
```

- [ ] **Step 6: 类型检查 + 实机验证**

Run: `npx tsc --noEmit` → 无输出

Run: `npm run tauri dev`：

1. 划词一个单词 → 卡片显示词典；点「对比各源」→ 逐行出现 有道词典 / 必应翻译；再点「返回词典」→ 回到词典卡片
2. 划词一整句 → 对比各源时「有道词典」行显示错误、「必应翻译」行显示译文（不是整页报错）
3. 点必应行的「固定为默认源」→ 变成「已固定」；再划同一个词 → 必应作为首选源被请求（可在设置页确认「首选源」= 必应翻译）
4. 设置页把「首选源」改回「自动」→ 单词重新走有道
5. 输入翻译窗口打开「对比各源」→ 开关后翻译 → 逐行结果；点某行复制 → 该行出现绿色对勾
6. 未配置百度密钥时对比页只有两行；配置后出现「百度翻译」行
7. 既有行为不回退：默认（未点对比）划词仍是单个词典卡片、延迟无明显变化

- [ ] **Step 7: 提交**

```bash
git add src/components/SourceCompare.tsx src/components/InputPage.tsx src/components/Popup.tsx \
  src/components/SettingsPage.tsx src/types.ts src/style.css
git commit -m "feat: 多翻译源对比（逐源译文、复制、固定首选源）"
```

---

### Task 4: README 更新

**Files:**
- Modify: `README.md`

- [ ] **Step 1: 更新文档**

- 功能列表加：「**多源对比**：一次查看有道 / 必应 / 百度译文，可逐行复制或固定首选源」
- 「翻译源配置」表格后补一段：「可选：在设置里把某个源固定为首选（或在对卡片里点『固定为默认源』）；固定源失败时仍按 有道 → 必应 → 百度 自动降级。」
- 使用说明补一条：「结果卡片或输入翻译窗口点『对比各源』可并排查看各源译文」

- [ ] **Step 2: 提交**

```bash
git add README.md
git commit -m "docs: README 补充多源对比说明"
```

---

## 验收清单（对应 issue #2 第 4 条）

- [ ] 同一段文本一次看到全部可用源的译文（含逐源错误行）
- [ ] 逐行独立复制；「固定为默认源」生效并持久化，设置页可改回「自动」
- [ ] 单源失败不影响其它源展示，也不影响整体结果返回
- [ ] 默认路径（不点对比）请求数与延迟与改造前一致
- [ ] `cargo test`、`npx tsc --noEmit` 通过

## 非目标（本期不做）

- 不做译文质量自动排序/打分（无可靠信号，且各源优劣与文本强相关）
- 不做「同一个源的多版本」对比（如百度通用/学术）
- 截图翻译卡片不接对比（原文可在输入翻译窗口里对比）
