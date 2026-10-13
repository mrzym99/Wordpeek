# 多翻译源对比与源优先级 实施计划（issue #2 第 4 条）

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 设置页勾选参与翻译的源并直接拖拽排序（顺序即优先级），输入翻译窗口与划词卡片一次看到各已选源的译文，逐行复制。

**Architecture:** 配置新增**有序**字段 `sources: Vec<String>`（默认 `["youdao","bing"]`，空/非法自动回退默认）；旧的「百度启用为主源」开关退役，其语义迁移为「把 baidu 排到 `sources` 最前」。单源快速路径（`build_order`）与对比路径（`compare_sources`）都由勾选顺序派生：前者按顺序降级，后者返回全部勾选源。新增 `translate_all` 命令并发逐源取结果（整体永不失败，失败源以错误行呈现），前端 `SourceCompare` 组件渲染逐源行。

**Tech Stack:** Tauri 2（`tauri::async_runtime::{spawn, JoinHandle}`，已核实 `Future::Output = tauri::Result<T>`）、reqwest 0.12、React 18（HTML5 拖拽，无新依赖）。

**Spec:** `https://github.com/mrzym99/Wordpeek/issues/2` 第 4 条；依赖 `2026-10-13-input-translate.md`（输入窗口、`to` 参数、`patchConfig`）与 `2026-10-13-popup-autosize.md`（卡片自适应高度）。

## 设计决策

| 决策 | 选择 | 理由 / 否决的替代方案 |
|---|---|---|
| 源的选择与优先级 | 设置页「翻译源」勾选列表 + 拖拽排序，顺序即优先级 | 用户明确要求；一个控件同时表达「用哪些源」和「谁优先」，不再需要单独的「启用为主源」开关，也不需要卡片上的「固定为默认源」按钮 |
| 只用勾选的源 | 单源路径只在勾选源内按顺序降级；**未勾选的源不再兜底** | 勾选是显式取舍（例如就是不想要必应）；静默回落到未勾选的源违背这个取舍。代价：勾选的源全失败就直接报错（错误信息逐源列出原因，可读） |
| 至少勾一个 | 列表禁止取消最后一个；后端 `enabled_sources` 也做空/非法回退默认 | 双保险，避免出现「没有可用源」的翻译请求 |
| 对比触发 | 输入翻译窗口直接展示全部勾选源（不再有「对比各源」开关）；划词卡片保留「对比各源」按钮 | 勾选本身就是选择，输入窗口不必再套一层开关；划词卡片默认走单源快速路径，避免每次划词都发 2~3 个请求 |
| 对比页位置 | 输入翻译窗口 + 划词卡片；截图卡片不做 | 截图卡片已有 OCR 原文编辑区，再塞多源结果太挤；原文可复制到输入翻译窗口对比 |
| 老配置迁移 | 旧 `source == "baidu"`（启用为主源）→ 迁移为 `sources` 首位是 baidu | 旧顺序等价于 `[baidu, youdao, bing]`；不迁移会让这些用户升级后百度突然不再参与翻译 |
| 并发方式 | 每源一个 `tauri::async_runtime::spawn` 任务，按固定顺序收集 | 不引 `futures` 依赖；`tokio::join!` 在源数量可变（1~3 个）时写起来很别扭 |
| 后端模型 | 新命令 `translate_all` 返回逐源结果（`Result<Vec<SourceResult>, String>`，恒 `Ok`），逐行 `ok/error` | 复用 `translate` 无法表达「部分失败」（如「有道这句没词条」）；单个源失败不该让整次对比失败 |

## Global Constraints

- 不新增依赖（Cargo 与 npm 均无新增）
- 新配置字段 `sources: Vec<String>` 用 `#[serde(default)]`（空 Vec = 未设置过），在 `load_config` 里迁移与兜底；旧字段 `source` 保留但**仅用于迁移**，标注 deprecated
- 现有行为不变：`translate` 命令签名与语义、`selection`/`screenshot-result` 事件、弹窗自动隐藏逻辑；**默认配置下顺序等价于改造前**（单词走有道、整句走必应）
- 注释与用户可见文案一律中文；禁止 TODO/TBD
- Rust 任务收尾必须 `cd src-tauri && cargo test` 通过；前端任务收尾必须 `npx tsc --noEmit` 通过
- 提交信息中文，格式 `feat|fix|test|docs: 描述`

---

### Task 1: 配置 `sources` + 老配置迁移 + 按勾选顺序的降级链

**Files:**
- Modify: `src-tauri/src/translate.rs`
- Modify: `src/types.ts`（`AppConfig.sources`）
- Test: `src-tauri/src/translate.rs` 的 `mod tests`

**Interfaces:**
- Produces:
  - `AppConfig.sources: Vec<String>`（有序；空 = 未设置）
  - `fn default_sources() -> Vec<String>`（`["youdao", "bing"]`）
  - `pub(crate) fn enabled_sources(cfg: &AppConfig) -> Vec<String>`（过滤非法名、去重、空则回退默认）
  - `fn migrate_sources(cfg: &mut AppConfig)`（老配置迁移，`load_config` 调用）
  - `fn push_source(order: &mut Vec<String>, source: &str, to: &str)`
  - `pub(crate) fn build_order(cfg: &AppConfig, to: &str) -> Vec<String>`
  - `translate_text(cfg, text, to)` 改为消费 `build_order`

- [ ] **Step 1: 写失败测试**

`translate.rs` 的 `mod tests` 追加（并保留原有测试）：

```rust
    #[test]
    fn migrate_sources_fills_defaults_for_old_configs() {
        // 老配置没有 sources 字段 → 空 Vec → 迁移补默认
        let mut cfg: AppConfig = serde_json::from_str("{}").unwrap();
        assert!(cfg.sources.is_empty());
        migrate_sources(&mut cfg);
        assert_eq!(cfg.sources, vec!["youdao", "bing"]);
        // 迁移幂等：已有值时不再动
        let mut again = cfg.clone();
        migrate_sources(&mut again);
        assert_eq!(again.sources, vec!["youdao", "bing"]);
    }

    #[test]
    fn legacy_baidu_primary_migrates_to_front() {
        // 旧「启用为主源」= source: "baidu"，等价于把百度排到最前
        let mut cfg: AppConfig = serde_json::from_str(
            r#"{"source":"baidu","baidu":{"appid":"a","secret":"b"}}"#,
        )
        .unwrap();
        migrate_sources(&mut cfg);
        assert_eq!(cfg.sources, vec!["baidu", "youdao", "bing"]);
    }

    #[test]
    fn enabled_sources_drops_unknown_and_duplicates() {
        let mut cfg = AppConfig::default();
        assert_eq!(enabled_sources(&cfg), vec!["youdao", "bing"]);
        cfg.sources = vec!["bing".into(), "nope".into(), "bing".into()];
        assert_eq!(enabled_sources(&cfg), vec!["bing"]);
        // 全非法 → 回退默认，绝不能返回空（否则翻译没有可用源）
        cfg.sources = vec!["nope".into()];
        assert_eq!(enabled_sources(&cfg), vec!["youdao", "bing"]);
    }

    #[test]
    fn build_order_follows_checked_order() {
        let mut cfg = AppConfig::default();
        cfg.sources = vec!["bing".into(), "youdao".into()];
        assert_eq!(build_order(&cfg, "zh"), vec!["bing", "youdao"]);
        // 译为英文时跳过有道（它只支持英→中）
        assert_eq!(build_order(&cfg, "en"), vec!["bing"]);
    }

    #[test]
    fn build_order_never_uses_unchecked_sources() {
        // 只勾必应：即使必应失败也不会回落到未勾选的有道
        let mut cfg = AppConfig::default();
        cfg.sources = vec!["bing".into()];
        assert_eq!(build_order(&cfg, "zh"), vec!["bing"]);
        // 勾了未配置密钥的百度：仍在队列里，由 translate_text 给出可读错误
        cfg.sources = vec!["baidu".into(), "bing".into()];
        assert_eq!(build_order(&cfg, "zh"), vec!["baidu", "bing"]);
    }
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cd src-tauri && cargo test migrate_sources enabled_sources build_order`

Expected: 编译失败（`sources` 字段、`migrate_sources`、`enabled_sources`、`build_order` 不存在）

- [ ] **Step 3: 实现**

`AppConfig`：给旧的 `source` 字段补注释（它只剩迁移用途），并在其后新增有序字段 `sources`：

```rust
    /// 旧字段：曾用于「百度启用为主源」，现仅作迁移依据（见 migrate_sources）
    #[serde(default = "default_source")]
    pub source: String,
    /// 参与翻译的源，按优先级排序（空 = 未设置过，加载时迁移补默认）
    #[serde(default)]
    pub sources: Vec<String>,
```

`impl Default for AppConfig` 里补一行 `sources: default_sources(),`（`source` 原样保留，才能读出老配置做迁移）；`src/types.ts` 的 `AppConfig` 补 `sources: string[];`。

`default_source()` 附近追加：

```rust
/// 默认参与翻译的源：单词走有道、整句走必应（与改造前一致）
fn default_sources() -> Vec<String> {
    vec!["youdao".into(), "bing".into()]
}

/// 剔除非法源名与重复项；空结果回退默认——任何情况下都不能返回空列表
pub(crate) fn enabled_sources(cfg: &AppConfig) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for s in &cfg.sources {
        if matches!(s.as_str(), "youdao" | "bing" | "baidu") && !out.contains(s) {
            out.push(s.clone());
        }
    }
    if out.is_empty() {
        return default_sources();
    }
    out
}

/// 老配置迁移：曾用 source = "baidu" 表达「百度作主源」，
/// 等价于把 baidu 排到列表最前（旧顺序即 [baidu, youdao, bing]）
fn migrate_sources(cfg: &mut AppConfig) {
    if !cfg.sources.is_empty() {
        return;
    }
    cfg.sources = default_sources();
    if cfg.source == "baidu" {
        cfg.sources.insert(0, "baidu".into());
    }
}
```

`load_config` 改为（解析成功后迁移）：

```rust
pub fn load_config(app: &AppHandle) -> AppConfig {
    let raw = config_path(app).and_then(|p| std::fs::read_to_string(p).ok());
    let mut cfg: AppConfig = raw
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    migrate_sources(&mut cfg);
    cfg
}
```

`push_source` / `build_order` / `translate_text`（替换 plan ② 那一版顺序构造）：

```rust
/// 把一个源按目标语言可用性追加进降级队列（去重）。
/// 未勾选的源不会进来：勾选即取舍。百度的密钥缺失留给 translate_text 报可读错误。
fn push_source(order: &mut Vec<String>, source: &str, to: &str) {
    let usable = match source {
        // 有道 jsonapi 只能英→中
        "youdao" => to == "zh",
        "bing" => true,
        "baidu" => true,
        _ => false,
    };
    if usable && !order.iter().any(|s| s == source) {
        order.push(source.to_string());
    }
}

/// 降级顺序 = 用户勾选顺序（译为英文时自动跳过有道）
pub(crate) fn build_order(cfg: &AppConfig, to: &str) -> Vec<String> {
    let mut order: Vec<String> = Vec::new();
    for s in enabled_sources(cfg) {
        push_source(&mut order, &s, to);
    }
    order
}

pub async fn translate_text(cfg: &AppConfig, text: &str, to: &str) -> Result<WordInfo, String> {
    let to = if to == "en" { "en" } else { "zh" };
    let baidu_ready = !cfg.baidu.appid.is_empty() && !cfg.baidu.secret.is_empty();
    let mut errors: Vec<String> = Vec::new();
    for src in build_order(cfg, to) {
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
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cd src-tauri && cargo test`

Expected: 全部 PASS（含既有 `default_strategy_falls_back_to_bing_without_baidu_keys`，证明默认配置行为未变）

- [ ] **Step 5: 提交**

```bash
git add src-tauri/src/translate.rs src/types.ts
git commit -m "feat: 翻译源改为有序勾选列表（sources），老配置自动迁移"
```

---

### Task 2: `translate_all` 并发对比命令

**Files:**
- Modify: `src-tauri/src/translate.rs`
- Modify: `src-tauri/src/main.rs`（`invoke_handler` 注册 `translate_all`）
- Test: `src-tauri/src/translate.rs` 的 `mod tests`

**Interfaces:**
- Consumes: Task 1 的 `enabled_sources`、`try_translate(source, text, baidu, to)`
- Produces:
  - `pub struct SourceResult { source: String, label: String, ok: bool, senses: Vec<String>, error: Option<String> }`
  - `pub(crate) fn source_label(source: &str) -> &'static str`
  - `pub(crate) fn compare_sources(cfg: &AppConfig, to: &str) -> Vec<String>`
  - `pub async fn translate_all_sources(cfg: &AppConfig, text: &str, to: &str) -> Vec<SourceResult>`
  - 命令 `translate_all(state, text: String, to: Option<String>) -> Result<Vec<SourceResult>, String>`（恒 `Ok`；带 `State` 引用的 async 命令必须返回 `Result`）

- [ ] **Step 1: 写失败测试**

```rust
    #[test]
    fn compare_sources_follows_checked_order_and_target() {
        let mut cfg = AppConfig::default();
        assert_eq!(compare_sources(&cfg, "zh"), vec!["youdao", "bing"]);
        // 译为英文：对比页也不该出现有道
        assert_eq!(compare_sources(&cfg, "en"), vec!["bing"]);
        // 顺序即用户拖拽出来的优先级
        cfg.sources = vec!["bing".into(), "youdao".into()];
        assert_eq!(compare_sources(&cfg, "zh"), vec!["bing", "youdao"]);
        // 勾了未配置密钥的百度：仍要出这一行（行内展示「未配置」原因）
        cfg.sources = vec!["bing".into(), "baidu".into()];
        assert_eq!(compare_sources(&cfg, "zh"), vec!["bing", "baidu"]);
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

Run: `cd src-tauri && cargo test compare_ source_labels`

Expected: 编译失败（`compare_sources` / `translate_all_sources` / `source_label` 不存在）

- [ ] **Step 3: 实现**

`translate_text` 之后插入：

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

/// 参与对比的源 = 用户勾选的源，按勾选顺序（译为英文时跳过有道）
pub(crate) fn compare_sources(cfg: &AppConfig, to: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for s in enabled_sources(cfg) {
        if s == "youdao" && to != "zh" {
            continue;
        }
        if !out.contains(&s) {
            out.push(s);
        }
    }
    out
}

/// 并发请求全部勾选源并逐源收集结果：整体不失败，单个源出错只影响它自己那一行
pub async fn translate_all_sources(cfg: &AppConfig, text: &str, to: &str) -> Vec<SourceResult> {
    let to = if to == "en" { "en" } else { "zh" };
    let baidu_ready = !cfg.baidu.appid.is_empty() && !cfg.baidu.secret.is_empty();
    // 每源一个异步任务：reqwest 请求并行发出，总耗时 ≈ 最慢的源
    let handles: Vec<(String, tauri::async_runtime::JoinHandle<Result<WordInfo, String>>)> =
        compare_sources(cfg, to)
            .into_iter()
            .map(|source| {
                let src = source.clone();
                let query = text.to_string();
                let baidu = cfg.baidu.clone();
                let to = to.to_string();
                let ready = baidu_ready;
                let handle = tauri::async_runtime::spawn(async move {
                    // 百度密钥缺失不必发请求：直接给出与单源路径一致的可读原因
                    if src == "baidu" && !ready {
                        return Err("未配置 appid/secret".to_string());
                    }
                    try_translate(&src, &query, &baidu, &to).await
                });
                (source, handle)
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

/// 对比命令：一次列出各勾选源译文（输入翻译窗口 / 划词卡片「对比各源」按钮）。
/// Tauri 要求带引用参数（State）的 async 命令必须返回 Result，这里恒为 Ok：
/// 单个源的失败已经在各自行里呈现，不该让整次对比失败。
#[tauri::command]
pub async fn translate_all(
    state: State<'_, SharedConfig>,
    text: String,
    to: Option<String>,
) -> Result<Vec<SourceResult>, String> {
    let cfg = state.lock().clone();
    Ok(translate_all_sources(&cfg, &text, to.as_deref().unwrap_or("zh")).await)
}
```

`main.rs` 的 `invoke_handler` 里，`translate::translate,` 之后追加 `translate::translate_all,`。

- [ ] **Step 4: 跑测试确认通过**

Run: `cd src-tauri && cargo test`

Expected: 全部 PASS（`compare_reports_each_source_separately` 走真实网络）

- [ ] **Step 5: 提交**

```bash
git add src-tauri/src/translate.rs src-tauri/src/main.rs
git commit -m "feat: 新增 translate_all 并发多源对比命令"
```

---

### Task 3: 设置页勾选 + 拖拽排序，输入窗口与划词卡片接入

**Files:**
- Create: `src/components/SourceCompare.tsx`
- Modify: `src/types.ts`（`SourceResult`）
- Modify: `src/components/SettingsPage.tsx`（翻译源勾选列表 + 拖拽排序；退役「启用为主源」开关）
- Modify: `src/components/InputPage.tsx`（直接展示逐源结果）
- Modify: `src/components/Popup.tsx`（「对比各源」/「返回词典」按钮）
- Modify: `src/style.css`（对比行与源列表样式）

**Interfaces:**
- Consumes: `translate_all({ text, to }) -> SourceResult[]`（Task 2）、`patchConfig`（`src/lib/config.ts`）、`get_config` 返回的 `sources`
- Produces: `<SourceCompare rows copied onCopy />`
  - `rows: SourceResult[]`、`copied: string`（命中的行显示对勾）
  - `onCopy: (source: string, text: string) => void`

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
  /** 复制反馈标记：命中的行显示对勾 */
  copied: string;
  onCopy: (source: string, text: string) => void;
}

/** 多源对比列表：每个已勾选源一行（顺序与设置页一致），含译文与复制 */
export default function SourceCompare({ rows, copied, onCopy }: SourceCompareProps) {
  return (
    <div className="compare">
      {rows.map((r) => (
        <div className="compare-row" key={r.source}>
          <div className="compare-head">
            <span className="compare-label">{r.label}</span>
            <button
              className={"icon-btn" + (copied === r.source ? " ok" : "")}
              title="复制该源译文"
              disabled={!r.ok}
              onClick={() => onCopy(r.source, r.senses.join("\n"))}
            >
              {copied === r.source ? <CheckIcon /> : <CopyIcon />}
            </button>
          </div>
          {r.ok ? <SenseList senses={r.senses} /> : <div className="compare-error">{r.error}</div>}
        </div>
      ))}
    </div>
  );
}
```

- [ ] **Step 2: 设置页改成勾选 + 拖拽排序列表**

`SettingsPage.tsx` 改动：

1. 文件顶部（`REPO_URL` 附近）加源定义：

```tsx
/** 可选翻译源（列表顺序由用户拖拽决定 = 优先级） */
const SOURCE_ITEMS: { id: string; label: string }[] = [
  { id: "youdao", label: "有道词典（免 Key，单词）" },
  { id: "bing", label: "必应翻译（免 Key，整句）" },
  { id: "baidu", label: "百度翻译（需密钥）" },
];
```

2. 状态：删除 `const [source, setSource] = useState("youdao");`，改为：

```tsx
  // 勾选的源，顺序即优先级（拖拽调整）
  const [sources, setSources] = useState<string[]>(["youdao", "bing"]);
  const [dragId, setDragId] = useState("");
```

3. 读配置的 effect 里把 `setSource(cfg.source);` 换成 `setSources(cfg.sources?.length ? cfg.sources : ["youdao", "bing"]);`

4. 删除 `baiduEnabled` / `enableBaidu` / `disableBaidu` 三个定义，新增：

```tsx
  // 勾选/取消：即时落盘；至少保留一个源
  const toggleSource = (id: string) => {
    const next = sources.includes(id)
      ? sources.filter((s) => s !== id)
      : [...sources, id];
    if (next.length === 0) return;
    setSources(next);
    patchConfig({ sources: next }).catch((e) => setSaveError(String(e)));
  };

  // 拖拽排序：把 from 插到 to 之前
  const moveSource = (from: string, to: string) => {
    if (!from || from === to) return;
    const next = sources.filter((s) => s !== from);
    const at = next.indexOf(to);
    next.splice(at < 0 ? next.length : at, 0, from);
    setSources(next);
    patchConfig({ sources: next }).catch((e) => setSaveError(String(e)));
  };
```

5. 「翻译」分组的渲染改为（替换原「百度翻译（可选）」整块：保留 APP ID / 密钥两个输入与「保存密钥」按钮，删掉「启用为主源」开关）：

```tsx
            <div className="section-heading">翻译</div>
            <div className="group-heading">翻译源（勾选参与翻译的源，拖拽调整优先级）</div>
            <div className="source-list">
              {sources.map((id) => {
                const item = SOURCE_ITEMS.find((s) => s.id === id);
                if (!item) return null;
                return (
                  <div
                    key={id}
                    className="source-item"
                    draggable
                    onDragStart={() => setDragId(id)}
                    onDragOver={(e) => e.preventDefault()}
                    onDrop={() => {
                      moveSource(dragId, id);
                      setDragId("");
                    }}
                  >
                    <span className="drag-handle" title="拖拽调整顺序">
                      ⠿
                    </span>
                    <label className="switch">
                      <input type="checkbox" checked onChange={() => toggleSource(id)} />
                      {item.label}
                    </label>
                  </div>
                );
              })}
              {SOURCE_ITEMS.filter((s) => !sources.includes(s.id)).map((s) => (
                <div key={s.id} className="source-item off">
                  <span className="drag-handle" />
                  <label className="switch">
                    <input type="checkbox" checked={false} onChange={() => toggleSource(s.id)} />
                    {s.label}
                  </label>
                </div>
              ))}
            </div>
            <div className="field-hint">
              顺序即优先级：第一个源也是单源快速翻译的首选，对比页按同一顺序展示；勾选百度需先填下面的密钥。未勾选的源不会参与翻译，也不会被用作失败兜底。
            </div>
            <div className="group-heading">百度翻译密钥（可选）</div>
            {/* ……APP ID / 密钥两个 label.field 与「保存密钥」按钮保持原样，仅删除「启用为主源」开关…… */}
```

6. `save()` 的 config 对象删掉 `source,`（其余字段保持；`...current` 会把旧 `source` 原样带回，后端只当迁移依据）。

- [ ] **Step 3: 输入翻译窗口直接展示逐源结果**

`InputPage.tsx` 改动（**替换** plan ② 里的单结果渲染）：

1. 把文件头的 import 整块换成下面这段（`WordInfo` / `SenseList` / `CheckIcon` / `CopyIcon` 都不再直接使用，图标与释义列表由 `SourceCompare` 内部引入）：

```tsx
import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { SourceResult } from "../types";
import { useAutoFitWindow } from "../lib/autofit";
import SourceCompare from "./SourceCompare";
```

2. 状态：`info` 换成 `rows`，`copied` 换成 `copiedKey`：

```tsx
  const [rows, setRows] = useState<SourceResult[] | null>(null);
  const [copiedKey, setCopiedKey] = useState("");
```

3. `run()` 改为：

```tsx
  const run = async () => {
    const q = text.trim();
    if (!q) return;
    setLoading(true);
    setError("");
    setRows(null);
    try {
      // 沿用文件里已有的 TARGET = "en"（固定译为英文）；参与对比的就是设置里勾选的源
      setRows(await invoke<SourceResult[]>("translate_all", { text: q, to: TARGET }));
    } catch (err) {
      setError(String(err));
    } finally {
      setLoading(false);
    }
  };
```

4. 复制回调改为按源标记：

```tsx
  const copyRow = (source: string, text: string) => {
    invoke("copy_text", { text })
      .then(() => {
        setCopiedKey(source);
        setTimeout(() => setCopiedKey(""), 1500);
      })
      .catch((e) => setError(String(e)));
  };
```

5. 结果区改为：

```tsx
        {loading && <div className="loading">翻译中…</div>}
        {rows && <SourceCompare rows={rows} copied={copiedKey} onCopy={copyRow} />}
```

- [ ] **Step 4: 划词卡片加「对比各源」按钮**

`Popup.tsx` 改动：

1. import 追加 `import SourceCompare from "./SourceCompare";`、把类型 import 补上 `AppConfig`… 不需要；只需 `SourceResult`：`import type { Selection, ShotResult, SourceResult, WordInfo } from "../types";`
2. `const [copied, setCopied] = useState("");` 类型放宽为 `useState<string>("")`，`doCopy` 的 `label` 参数类型由 `"text" | "translation"` 改为 `string`。
   `ShotCard` 不用改：它的 `onCopy` 形参类型更窄（`"text" | "translation"`），传一个形参类型更宽的 `doCopy` 进去是合法的（形参逆变）
3. 状态追加：

```tsx
  // 对比模式：非空时用多源结果替换词典卡片
  const [rows, setRows] = useState<SourceResult[] | null>(null);
  const [compareLoading, setCompareLoading] = useState(false);
```

4. `selection` 事件处理里 `setShotResult(null);` 之后追加 `setRows(null);`（新划词退出对比视图）
5. 新增：

```tsx
  const runCompare = async (text: string) => {
    setCompareLoading(true);
    setRows(null);
    setError("");
    try {
      setRows(await invoke<SourceResult[]>("translate_all", { text, to: "zh" }));
    } catch (err) {
      setError(String(err));
    } finally {
      setCompareLoading(false);
    }
  };
```

6. `.word-head` 里，齿轮按钮之前加：

```tsx
        {word && (
          <button
            className="text-btn"
            title={rows ? "返回词典卡片" : "同时查看各已勾选源的译文"}
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

      {rows && <SourceCompare rows={rows} copied={copied} onCopy={doCopy} />}

      {!rows && info && (
        <>
          {/* ……原有 senses / word_forms / synos 渲染保持不变…… */}
        </>
      )}
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
.compare-row .senses { margin-top: 2px; }
.compare-error {
  margin-top: 4px;
  color: var(--error);
  font-size: 12.5px;
  white-space: pre-wrap;
}

/* ===== 设置页：翻译源勾选 + 拖拽排序 ===== */
.source-list { margin-bottom: 6px; }
.source-item {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 3px 4px;
  border-radius: 4px;
  user-select: none; /* 拖拽时别选中文本 */
}
.source-item:hover { background: var(--bg-subtle); }
.source-item.off { color: var(--text-faint); }
.drag-handle {
  width: 14px;
  color: var(--text-faint);
  cursor: grab;
}

/* 文字按钮：卡片/设置页里的小动作（对比各源） */
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
```

- [ ] **Step 6: 类型检查 + 实机验证**

Run: `npx tsc --noEmit` → 无输出

Run: `npm run tauri dev`：

1. 设置 → 翻译：默认勾选「有道词典」「必应翻译」，顺序即优先级；取消勾选必应 → 只剩有道（单源）
2. 拖拽把「必应翻译」拖到「有道词典」之前 → 松手后顺序变化，重开设置/重启后保持
3. 勾选百度但没填密钥 → 输入窗口对比行显示「未配置 appid/secret」；填上密钥保存后该行出译文
4. 输入翻译窗口：粘贴中文 → 直接看到各勾选源的英文译文（无需再点开关）；某源失败只影响它那一行
5. 划词一个单词 → 词典卡片；点「对比各源」→ 逐行结果；「返回词典」回退
6. 划词一整句 → 对比时「有道词典」行报错、「必应翻译」行出译文（不是整页报错）
7. 未勾选的源确实不参与：只勾必应 → 划词单词也只出必应译文（不再回落有道）
8. 逐行复制 → 该行出现绿色对勾

- [ ] **Step 7: 提交**

```bash
git add src/components/SourceCompare.tsx src/components/InputPage.tsx src/components/Popup.tsx \
  src/components/SettingsPage.tsx src/types.ts src/style.css
git commit -m "feat: 翻译源勾选+拖拽排序，输入窗口与划词卡片支持多源对比"
```

---

### Task 4: README 更新

**Files:**
- Modify: `README.md`

- [ ] **Step 1: 更新文档**

- 功能列表把「多翻译源」一条改为：「**多翻译源（可选勾选 + 拖拽排序）**：设置里勾选参与翻译的源（有道词典 / 必应翻译 / 百度翻译）并拖拽调整优先级；未勾选的源不参与、也不作失败兜底」
- 加一条：「**多源对比**：输入翻译窗口直接并排展示各已勾选源的译文，划词卡片点『对比各源』也可查看，逐行可复制」
- 「翻译源配置」表格后的默认策略段改写：「默认策略：**勾选有道词典 + 必应翻译**，单词走有道、整句自动走必应；百度翻译需在设置里填 appid + 密钥并勾选。顺序即优先级（第一个源也是失败降级的首个候选）。」
- 使用说明补一条：「结果卡片或输入翻译窗口可查看多源译文对比」

- [ ] **Step 2: 提交**

```bash
git add README.md
git commit -m "docs: README 改为按勾选源说明翻译与对比"
```

---

## 验收清单（对应 issue #2 第 4 条）

- [ ] 设置页可勾选翻译源、可拖拽排序，顺序即优先级且持久化
- [ ] 未勾选的源不参与翻译、也不作兜底；最后一个源不可取消
- [ ] 输入翻译窗口直接给出各勾选源的译文；划词卡片「对比各源」可用
- [ ] 单源失败不影响其它源展示，也不影响整体结果返回
- [ ] 老配置升级后行为合理：曾启用「百度为主源」的用户，升级后百度仍在优先级首位
- [ ] 默认配置（未动设置）下与改造前等价：单词走有道、整句走必应
- [ ] `cargo test`、`npx tsc --noEmit` 通过

## 非目标（本期不做）

- 不做译文质量自动排序/打分（无可靠信号，各源优劣与文本强相关）
- 不做「同一个源的多版本」对比（如百度通用/学术）
- 截图翻译卡片不接对比（原文可复制到输入翻译窗口对比）
- 不做拖拽排序的键盘替代操作（如需无障碍支持，后续加「上移/下移」按钮）
