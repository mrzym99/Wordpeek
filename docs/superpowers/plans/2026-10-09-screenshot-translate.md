# 截图翻译 实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 全局快捷键触发 → 全屏定格 → 框选区域 → Windows OCR → 复用现有翻译管线 → 双语卡片。

**Architecture:** Rust 端 GDI 抓取虚拟屏幕（多屏一张图）→ screenshot 遮罩窗口铺满虚拟屏幕 → 前端拖拽框选 → Rust 裁剪 + PNG 编码 → 后台线程 Windows.Media.Ocr → `translate_text`（有道→百度降级）→ 复用 main 弹窗显示双语卡片。OCR 结果保留行坐标（本期不使用，为原位覆盖铺路）。

**Tech Stack:** Tauri 2 + React 18 + TypeScript（现有栈）；Rust 新增：windows 0.58（增开 GDI/WinRT features）、image 0.25（仅 png）、base64 0.22。

**Spec:** `docs/superpowers/specs/2026-10-09-screenshot-translate-design.md`（须与 spec 一并阅读）

## Global Constraints

- 仅 Windows：GDI/OCR 代码用 `#[cfg(windows)]` 门控；纯函数（crop/编码/语言匹配）跨平台可测
- 新增依赖仅：`image = { version = "0.25", default-features = false, features = ["png"] }`、`base64 = "0.22"`；`windows` crate 只增开 features
- 不改变现有划词行为：事件 `"selection"`、命令 `translate`/`get_config`/`save_config`、窗口结构均不动
- 快捷键默认值：划词 `Ctrl+Alt+T`（不变）、截图 `Ctrl+Alt+S`；两者不可相同
- 注释与错误信息一律中文；禁止 TODO/TBD
- Rust 任务收尾：`src-tauri/` 下 `cargo test` 必须通过；前端任务收尾：`npx tsc --noEmit` 必须通过
- 提交信息中文，格式 `feat|fix|test|docs: 描述`

---

### Task 1: `capture.rs` — GDI 抓屏 + 裁剪 + PNG 编码

**Files:**
- Create: `src-tauri/src/capture.rs`
- Modify: `src-tauri/src/main.rs`（`mod translate;` 后加 `mod capture;`）
- Modify: `src-tauri/Cargo.toml`（依赖与 windows features）

**Interfaces:**
- Consumes: 无
- Produces:
  - `pub struct Rect { pub x: i32, pub y: i32, pub w: i32, pub h: i32 }`
  - `pub struct CapturedScreen { pub width: i32, pub height: i32, pub origin_x: i32, pub origin_y: i32, pub rgba: Vec<u8> }`
  - `pub fn capture_virtual_screen() -> Result<CapturedScreen, String>`（仅 Windows）
  - `pub fn crop(rgba: &[u8], width: i32, height: i32, rect: Rect) -> Result<Vec<u8>, String>`（纯函数）
  - `pub fn encode_png(rgba: &[u8], width: i32, height: i32) -> Result<Vec<u8>, String>`（纯函数）

- [ ] **Step 1: 加依赖**

`src-tauri/Cargo.toml` 的 `windows` 依赖行替换为：

```toml
windows = { version = "0.58", features = ["Win32_UI_WindowsAndMessaging", "Win32_Foundation", "Win32_Graphics_Gdi"] }
```

`[dependencies]` 追加：

```toml
image = { version = "0.25", default-features = false, features = ["png"] }  # 仅 PNG 编码
```

- [ ] **Step 2: 写失败测试 + 类型骨架**

创建 `src-tauri/src/capture.rs`（顶部 `#![allow(dead_code)]` 暂放，Task 5 接入后移除），先写类型与测试：

```rust
#![allow(dead_code)] // Task 5 接入截图流程后移除
//! GDI 抓屏：虚拟屏幕截取、裁剪、PNG 编码（抓屏仅 Windows，其余纯函数跨平台）

/// 选区（物理像素，相对虚拟屏幕原点）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

/// 一次定格的整屏抓取结果
pub struct CapturedScreen {
    pub width: i32,
    pub height: i32,
    pub origin_x: i32,
    pub origin_y: i32,
    /// RGBA，按行排列
    pub rgba: Vec<u8>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 2x2 图像：(0,0)红 (1,0)绿 (0,1)蓝 (1,1)白
    fn sample() -> (Vec<u8>, i32, i32) {
        (
            vec![255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255],
            2,
            2,
        )
    }

    #[test]
    fn crop_takes_inner_region() {
        let (rgba, w, h) = sample();
        let out = crop(&rgba, w, h, Rect { x: 1, y: 1, w: 1, h: 1 }).expect("应裁剪成功");
        assert_eq!(out, vec![255, 255, 255, 255]); // 白色像素
    }

    #[test]
    fn crop_clamps_out_of_bounds() {
        let (rgba, w, h) = sample();
        // 越界选区被钳制到图像内
        let out = crop(&rgba, w, h, Rect { x: 1, y: 1, w: 9, h: 9 }).expect("应裁剪成功");
        assert_eq!(out, vec![255, 255, 255, 255]);
    }

    #[test]
    fn crop_rejects_empty_selection() {
        let (rgba, w, h) = sample();
        assert!(crop(&rgba, w, h, Rect { x: 0, y: 0, w: 0, h: 5 }).is_err());
        assert!(crop(&rgba, w, h, Rect { x: 2, y: 0, w: 4, h: 4 }).is_err());
    }

    #[test]
    fn png_roundtrip_decodes_same_pixels() {
        let (rgba, w, h) = sample();
        let png = encode_png(&rgba, w, h).expect("编码应成功");
        let img = image::load_from_memory(&png).expect("解码应成功");
        assert_eq!((img.width(), img.height()), (2, 2));
        assert_eq!(img.to_rgba8().into_raw(), rgba);
    }
}
```

- [ ] **Step 3: 实现最小代码（使测试通过）**

`capture.rs` 测试模块之前追加实现：

```rust
/// 裁剪 RGBA 图像中的选区；越界部分钳制到图像内，空选区报错
pub fn crop(rgba: &[u8], width: i32, height: i32, rect: Rect) -> Result<Vec<u8>, String> {
    if width <= 0 || height <= 0 {
        return Err("图像尺寸非法".into());
    }
    let x = rect.x.clamp(0, width);
    let y = rect.y.clamp(0, height);
    let w = rect.w.clamp(0, width - x);
    let h = rect.h.clamp(0, height - y);
    if w == 0 || h == 0 {
        return Err("选区为空".into());
    }
    let stride = width as usize * 4;
    let row_len = w as usize * 4;
    let mut out = Vec::with_capacity(row_len * h as usize);
    for row in 0..h as usize {
        let start = (y as usize + row) * stride + x as usize * 4;
        out.extend_from_slice(&rgba[start..start + row_len]);
    }
    Ok(out)
}

/// RGBA 原始像素编码为 PNG
pub fn encode_png(rgba: &[u8], width: i32, height: i32) -> Result<Vec<u8>, String> {
    if width <= 0 || height <= 0 {
        return Err("图像尺寸非法".into());
    }
    let mut out = Vec::new();
    image::codecs::png::PngEncoder::new(std::io::Cursor::new(&mut out))
        .encode(rgba, width as u32, height as u32, image::ExtendedColorType::Rgba8)
        .map_err(|e| format!("PNG 编码失败: {e}"))?;
    Ok(out)
}

#[cfg(windows)]
pub fn capture_virtual_screen() -> Result<CapturedScreen, String> {
    use windows::Win32::Graphics::Gdi::{
        BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC,
        GetDIBits, ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS, SRCCOPY,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN,
        SM_YVIRTUALSCREEN,
    };

    unsafe {
        let ox = GetSystemMetrics(SM_XVIRTUALSCREEN);
        let oy = GetSystemMetrics(SM_YVIRTUALSCREEN);
        let w = GetSystemMetrics(SM_CXVIRTUALSCREEN);
        let h = GetSystemMetrics(SM_CYVIRTUALSCREEN);
        if w <= 0 || h <= 0 {
            return Err("获取虚拟屏幕尺寸失败".into());
        }

        let screen_dc = GetDC(None);
        let mem_dc = CreateCompatibleDC(Some(screen_dc));
        let bitmap = CreateCompatibleBitmap(screen_dc, w, h);
        let old = SelectObject(mem_dc, bitmap.into());
        let blit = BitBlt(mem_dc, 0, 0, w, h, Some(screen_dc), ox, oy, SRCCOPY);

        // 负高度 = 自上而下行序；32bpp BI_RGB 输出为 BGRA
        let mut bmi = BITMAPINFO::default();
        bmi.bmiHeader = BITMAPINFOHEADER {
            biWidth: w,
            biHeight: -h,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: 0, // BI_RGB
            ..Default::default()
        };
        let mut rgba = vec![0u8; (w as usize) * (h as usize) * 4];
        let lines = GetDIBits(
            mem_dc,
            bitmap,
            0,
            h as u32,
            Some(rgba.as_mut_ptr() as *mut _),
            &mut bmi,
            DIB_RGB_COLORS,
        );

        SelectObject(mem_dc, old);
        let _ = DeleteObject(bitmap.into());
        let _ = DeleteDC(mem_dc);
        ReleaseDC(None, screen_dc);

        // windows 0.58 中 BitBlt 返回 BOOL；若本地版本返回 Result 则按编译器提示改用 is_err()
        if !blit.as_bool() || lines == 0 {
            return Err("BitBlt/GetDIBits 抓屏失败".into());
        }
        bgra_to_rgba(&mut rgba);
        Ok(CapturedScreen { width: w, height: h, origin_x: ox, origin_y: oy, rgba })
    }
}

#[cfg(not(windows))]
pub fn capture_virtual_screen() -> Result<CapturedScreen, String> {
    Err("截图翻译仅支持 Windows".into())
}

/// BGRA（GDI 输出）→ RGBA，alpha 置 255
fn bgra_to_rgba(buf: &mut [u8]) {
    for px in buf.chunks_exact_mut(4) {
        px.swap(0, 2);
        px[3] = 255;
    }
}
```

- [ ] **Step 4: 声明模块**

`src-tauri/src/main.rs` 第 16 行 `mod translate;` 后加：

```rust
mod capture;
```

- [ ] **Step 5: 跑测试确认通过**

```bash
cd src-tauri && cargo test capture
```

预期：4 个 `capture::tests` 全部 PASS，既有测试（parse_common_hotkeys 等）不受影响。

- [ ] **Step 6: 提交**

```bash
git add src-tauri/src/capture.rs src-tauri/src/main.rs src-tauri/Cargo.toml src-tauri/Cargo.lock
git commit -m "feat: GDI 抓屏模块（虚拟屏截取/裁剪/PNG 编码）"
```

---


---

### Task 2: `ocr.rs` — Windows.Media.Ocr 封装

**Files:**
- Create: `src-tauri/src/ocr.rs`
- Modify: `src-tauri/Cargo.toml`（windows features 增开）
- Modify: `src-tauri/src/main.rs`（`mod capture;` 后加 `mod ocr;`）

**Interfaces:**
- Consumes: PNG 字节（Task 1 的 `encode_png` 产出）
- Produces:
  - `pub struct OcrLine { pub text: String, pub x: i32, pub y: i32, pub w: i32, pub h: i32 }`（坐标相对裁剪图，本期不使用）
  - `pub fn recognize(png: &[u8], lang: &str) -> Result<Vec<OcrLine>, String>`（lang: `"auto"` | `"zh"` | `"en"`，仅 Windows 有实现）
  - `fn pick_language(available: &[String], lang: &str) -> Option<String>`（纯函数）

- [ ] **Step 1: 增开 windows features**

`src-tauri/Cargo.toml` 的 windows 依赖行替换为：

```toml
windows = { version = "0.58", features = [
    "Win32_UI_WindowsAndMessaging", "Win32_Foundation", "Win32_Graphics_Gdi",
    "Foundation", "Globalization", "Graphics_Imaging", "Media_Ocr", "Storage_Streams",
] }
```

- [ ] **Step 2: 纯函数 pick_language 与测试（一次落地）**

创建 `src-tauri/src/ocr.rs`：

```rust
#![allow(dead_code)] // Task 5 接入截图流程后移除
//! Windows.Media.Ocr：PNG → 文本行（行坐标保留，本期不使用）

/// OCR 识别出的一行文字；坐标为相对裁剪图像的物理像素
#[derive(Debug, Clone)]
pub struct OcrLine {
    pub text: String,
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

/// 从系统可用语言标签里挑识别语言。
/// "zh" 优先 zh-Hans（简体），退而求其次任意 zh 开头；
/// "en" 优先 en-US，其次任意 en；"auto" 返回 None（引擎按用户语言创建）
fn pick_language(available: &[String], lang: &str) -> Option<String> {
    match lang {
        "zh" => available
            .iter()
            .find(|a| a.to_lowercase().starts_with("zh-hans"))
            .or_else(|| available.iter().find(|a| a.to_lowercase().starts_with("zh")))
            .cloned(),
        "en" => available
            .iter()
            .find(|a| a.eq_ignore_ascii_case("en-us"))
            .or_else(|| available.iter().find(|a| a.to_lowercase().starts_with("en")))
            .cloned(),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zh_prefers_simplified() {
        let langs = vec!["en-US".into(), "zh-Hant".into(), "zh-Hans".into()];
        assert_eq!(pick_language(&langs, "zh"), Some("zh-Hans".into()));
    }

    #[test]
    fn zh_falls_back_to_any_chinese() {
        let langs = vec!["zh-Hant-TW".into()];
        assert_eq!(pick_language(&langs, "zh"), Some("zh-Hant-TW".into()));
    }

    #[test]
    fn en_prefers_us() {
        let langs = vec!["en-GB".into(), "en-US".into()];
        assert_eq!(pick_language(&langs, "en"), Some("en-US".into()));
    }

    #[test]
    fn auto_or_no_match_returns_none() {
        let langs = vec!["fr-FR".into()];
        assert_eq!(pick_language(&langs, "zh"), None);
        assert_eq!(pick_language(&langs, "auto"), None);
    }
}
```

- [ ] **Step 3: 跑测试确认通过**

```bash
cd src-tauri && cargo test ocr
```

预期：4 个 `ocr::tests` 通过。pick_language 是纯函数、实现与测试同文件落地；`recognize` 依赖系统 OCR 引擎，不做自动化单测（Task 7 手动验证）。

---

### Task 3: `translate.rs` 重构 — `translate_text` 抽取 + 有道整句分支

**Files:**
- Modify: `src-tauri/src/translate.rs`（`translate_youdao` 换行分支、`translate` 命令瘦身、新增 `translate_text`）

**Interfaces:**
- Produces:
  - `pub async fn translate_text(cfg: &AppConfig, text: &str) -> Result<WordInfo, String>`（主源→降级顺序不变；Task 6 截图管线调用）
  - `translate` 命令签名不变：`pub async fn translate(state: State<'_, SharedConfig>, text: String) -> Result<WordInfo, String>`

- [ ] **Step 1: 写失败的整句测试**

`translate.rs` 的 `#[cfg(test)] mod tests` 内追加（有道整句走 `fanyi.tran` 节点，现走不到会 Err）：

```rust
#[tokio::test]
async fn youdao_translates_full_sentence() {
    // 整句没有词条 ec.word，应回落到 fanyi.tran 而不是报"未找到释义"
    let info = translate_youdao("The quick brown fox jumps over the lazy dog")
        .await
        .expect("整句应能翻译");
    assert!(!info.senses.is_empty());
    assert_eq!(info.source, "youdao");
}
```

- [ ] **Step 2: 跑测试确认失败**

```bash
cd src-tauri && cargo test youdao_translates_full_sentence
```

预期：`Err("未找到释义")` 断言失败（panic: called `Result::expect()`）。

- [ ] **Step 3: 实现 `translate_youdao` 整句回落**

`translate_youdao` 中 `let w = &resp["ec"]["word"][0]; if w.is_null() { return Err("未找到释义".into()); }` 替换为：

```rust
    let w = &resp["ec"]["word"][0];
    if w.is_null() {
        // 整句/长句没有词条：回落到 fanyi.tran（有道整句翻译节点）
        let tran = resp["fanyi"]["tran"].as_str().unwrap_or_default();
        if !tran.trim().is_empty() {
            return Ok(WordInfo {
                word: text.to_string(),
                source: "youdao".into(),
                senses: vec![tran.to_string()],
                ..Default::default()
            });
        }
        return Err("未找到释义".into());
    }
```

- [ ] **Step 4: 跑测试确认通过**

```bash
cd src-tauri && cargo test youdao
```

预期：3 个既有 youdao 测试 + 新测试全部 PASS。

- [ ] **Step 5: 抽取 `translate_text`（纯重构）**

`translate.rs` 中 `/// 用配置的主源翻译…` 注释下的整个 `translate` 命令（约第 277-306 行）替换为：

```rust
/// 翻译核心：主源优先，失败按 有道→百度 降级（百度需已配置 key）。
/// 从 translate 命令抽出，供截图翻译管线在无 State 的上下文直接调用。
pub async fn translate_text(cfg: &AppConfig, text: &str) -> Result<WordInfo, String> {
    // 主源排最前，其余作为降级候选
    let mut order: Vec<String> = vec![cfg.source.clone()];
    for s in ["youdao", "baidu"] {
        if !order.contains(&s.to_string()) {
            order.push(s.to_string());
        }
    }

    let mut errors: Vec<String> = Vec::new();
    for src in order {
        if src == "baidu" && (cfg.baidu.appid.is_empty() || cfg.baidu.secret.is_empty()) {
            errors.push("[百度] 未配置 appid/secret".into());
            continue;
        }
        match try_translate(&src, text, &cfg.baidu).await {
            Ok(info) => return Ok(info),
            Err(e) => errors.push(format!("[{src}] {e}")),
        }
    }
    Err(errors.join("；"))
}

#[tauri::command]
pub async fn translate(state: State<'_, SharedConfig>, text: String) -> Result<WordInfo, String> {
    let cfg = state.lock().unwrap().clone();
    translate_text(&cfg, &text).await
}
```

- [ ] **Step 6: 跑全部测试 + 提交**

```bash
cd src-tauri && cargo test
git add src-tauri/src/translate.rs
git commit -m "refactor: 抽取 translate_text 供截图管线复用；有道补整句翻译分支"
```

预期：全部测试 PASS；划词行为不变。

---

### Task 4: `AppConfig` 新字段 — `screenshot_hotkey` + `ocr_lang`

**Files:**
- Modify: `src-tauri/src/translate.rs`（`AppConfig` 加字段、默认值、`Default` impl；`save_config` 同步透传）

**Interfaces:**
- Produces:
  - `AppConfig` 新增 `pub screenshot_hotkey: String`（默认 `"Ctrl+Alt+S"`）、`pub ocr_lang: String`（`"auto" | "zh" | "en"`，默认 `"auto"`）
  - Task 5 的 `register_hotkeys`、Task 9 的设置页均消费这两个字段

- [ ] **Step 1: 写失败测试（serde 默认值回填）**

`translate.rs` 的 `mod tests` 追加：

```rust
#[test]
fn config_defaults_fill_screenshot_fields() {
    // 旧配置文件没有新字段：serde 用默认值补齐，向后兼容
    let cfg: AppConfig = serde_json::from_str("{}").unwrap();
    assert_eq!(cfg.screenshot_hotkey, "Ctrl+Alt+S");
    assert_eq!(cfg.ocr_lang, "auto");
}
```

- [ ] **Step 2: 跑测试确认失败**

```bash
cd src-tauri && cargo test config_defaults
```

预期：编译错误 `no field screenshot_hotkey`。

- [ ] **Step 3: 实现字段与默认值**

`AppConfig` 结构体 `pub hotkey` 字段后追加：

```rust
    /// 截图翻译快捷键，如 "Ctrl+Alt+S"
    #[serde(default = "default_screenshot_hotkey")]
    pub screenshot_hotkey: String,
    /// OCR 识别语言："auto" | "zh" | "en"
    #[serde(default = "default_ocr_lang")]
    pub ocr_lang: String,
```

`default_hotkey` 函数后追加两个默认值函数，并同步 `impl Default for AppConfig`：

```rust
fn default_screenshot_hotkey() -> String {
    "Ctrl+Alt+S".into()
}

fn default_ocr_lang() -> String {
    "auto".into()
}
```

`impl Default for AppConfig` 块内 `hotkey: default_hotkey(),` 后追加：

```rust
            screenshot_hotkey: default_screenshot_hotkey(),
            ocr_lang: default_ocr_lang(),
```

- [ ] **Step 4: 跑测试确认通过 + 提交**

```bash
cd src-tauri && cargo test
git add src-tauri/src/translate.rs
git commit -m "feat: 配置新增截图快捷键与 OCR 语言字段（旧配置自动补默认值）"
```

预期：全部测试 PASS。

---

### Task 5a: 截图窗口与能力配置（M1 基础设施）

**Files:**
- Modify: `src-tauri/tauri.conf.json`（`windows` 数组加 screenshot 窗口；CSP 加 `img-src`）
- Modify: `src-tauri/capabilities/default.json`（`windows` 加 `"screenshot"`）

- [ ] **Step 1: tauri.conf.json**

`app.windows` 数组（`settings` 项之后）追加：

```json
      {
        "label": "screenshot",
        "title": "Wordpeek 截图",
        "url": "/?page=screenshot",
        "width": 800,
        "height": 600,
        "decorations": false,
        "transparent": true,
        "alwaysOnTop": true,
        "skipTaskbar": true,
        "visible": false,
        "resizable": false,
        "shadow": false
      }
```

`app.security.csp` 替换为（追加 `img-src 'self' data:`）：

```json
"csp": "default-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; media-src https://dict.youdao.com"
```

- [ ] **Step 2: capabilities/default.json**

`"windows": ["main"]` → `"windows": ["main", "screenshot"]`（权限集不变）。

- [ ] **Step 3: 验证配置可解析**

```bash
cd src-tauri && cargo check
```

预期：`tauri.conf.json` 通过 schema 校验，编译无错。

- [ ] **Step 4: 提交**

```bash
git add src-tauri/tauri.conf.json src-tauri/capabilities/default.json
git commit -m "feat: 新增 screenshot 遮罩窗口与 CSP img-src 配置"
```

---

### Task 5b: `screenshot.rs` — 截图流程状态机与 M1 数据流

**Files:**
- Create: `src-tauri/src/screenshot.rs`
- Modify: `src-tauri/src/main.rs`（`mod screenshot;`、`State` 注册、`screenshot::handle_trigger` 接入热键、托盘菜单）

**Interfaces:**
- Consumes: Task 1 `capture::{capture_virtual_screen, crop, encode_png}`；Task 2 `ocr::recognize`（M2 接入）；Task 4 `AppConfig`
- Produces:
  - 事件 `screenshot-start { data_url, width, height }`（仅发给 `screenshot` 窗口，base64 PNG data URL）
  - 事件 `screenshot-pending { x, y }`（裁剪区左上角，物理坐标；`main` 窗口接）
  - 事件 `screenshot-result { ok, text, error }`（M1 写临时 PNG 验证裁剪闭环）
  - 命令 `screenshot_finish(x, y, w, h)` / `screenshot_cancel()`

**数据流（M1）:**

```
热键/托盘 → handle_trigger():
  隐藏 settings（记录 was_visible）→ capture_virtual_screen()
  screenshot 窗口 set_position/set_size（虚拟屏原点+尺寸，物理像素）
  状态存 Mutex<ScreenshotState>（rgba + origin + was_visible）→ show + set_focus
  → emit "screenshot-start"（data URL）
前端: <img> 铺满 → 拖拽框选 → invoke("screenshot_finish", {x,y,w,h})（CSS×scaleFactor 取整）
ESC/blur → invoke("screenshot_cancel")
finish: hide screenshot 窗口 → crop → encode_png → 写 %TEMP%\wordpeek-crop.png（M1 验证用）
        → M2 起改为: ocr::recognize → emit pending/result
cancel: hide → 恢复 settings 可见性 → 清空状态
```

- [ ] **Step 1: `screenshot.rs` 骨架（M1 可运行）**

```rust
//! 截图翻译流程：触发 → 遮罩框选 → 裁剪 →（M1: 临时 PNG；M2: OCR→翻译→事件）

use std::sync::Mutex;
use base64::Engine;
use tauri::{AppHandle, Emitter, Manager};

use crate::capture::{self, CapturedScreen};

#[derive(Default)]
pub struct ScreenshotState {
    /// 本次定格的全屏图（finish 时 take）
    pub screen: Option<CapturedScreen>,
    /// 触发时 settings 窗口是否可见（结束后恢复）
    pub settings_was_visible: bool,
}

#[derive(serde::Serialize, Clone)]
struct ScreenshotStart {
    data_url: String,
    width: i32,
    height: i32,
}

pub fn handle_trigger(app: &AppHandle) {
    let app = app.clone();
    std::thread::spawn(move || {
        if let Err(e) = run_trigger(&app) {
            eprintln!("[screenshot] {e}");
        }
    });
}

fn run_trigger(app: &AppHandle) -> Result<(), String> {
    let screen = capture::capture_virtual_screen()?;
    let png = capture::encode_png(&screen.rgba, screen.width, screen.height)?;
    let data_url = format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(&png));

    // 记录并隐藏 settings，避免遮罩盖住时用户误操作
    let settings = app.get_webview_window("settings");
    {
        let mut st = app.state::<Mutex<ScreenshotState>>().lock().unwrap();
        st.screen = Some(screen);
        st.settings_was_visible = settings
            .as_ref()
            .map(|w| w.is_visible().unwrap_or(false))
            .unwrap_or(false);
    }
    if let Some(w) = &settings {
        if st_is_visible_set(&app) {
            let _ = w.hide();
        }
    }
    let st = app.state::<Mutex<ScreenshotState>>().lock().unwrap();
    let screen = st.screen.as_ref().ok_or("状态已被占用")?;
    let (ow, oh) = (screen.width, screen.height);
    let (ox, oy) = (screen.origin_x, screen.origin_y);
    drop(st);

    let win = app
        .get_webview_window("screenshot")
        .ok_or("找不到 screenshot 窗口")?;
    win.set_position(tauri::PhysicalPosition::new(ox, oy))
        .map_err(|e| e.to_string())?;
    win.set_size(tauri::PhysicalSize::new(ow as u32, oh as u32))
        .map_err(|e| e.to_string())?;
    win.show().map_err(|e| e.to_string())?;
    win.set_focus().map_err(|e| e.to_string())?;
    app.emit_to(
        "screenshot",
        "screenshot-start",
        ScreenshotStart { data_url, width: ow, height: oh },
    )
    .map_err(|e| e.to_string())
}

fn st_is_visible_set(app: &AppHandle) -> bool {
    app.state::<Mutex<ScreenshotState>>()
        .lock()
        .unwrap()
        .settings_was_visible
}
T6X

---

### Task 6: 前端遮罩页 `ScreenshotPage.tsx`（框选交互）

**Files:** Create `src/ScreenshotPage.tsx`; Modify `src/App.tsx`（`page === "screenshot"` → `<ScreenshotPage />`，与 settings 路由并列）、`src/style.css`

**Interfaces:** 消费事件 `screenshot-start { data_url, width, height }`（width/height 为物理像素）；命令 `screenshot_finish({ x, y, w, h })`、`screenshot_cancel()`（均为物理像素）

- [ ] `ScreenshotPage`：`useState<{ url: string } | null>`；mount 时 `listen<ScreenshotStart>("screenshot-start", e => setShot(e.payload))`
- [ ] `shot` 存在时渲染：`.shot-mask`（fixed inset-0）内 `<img src={url} className="shot-frame" draggable={false} />`（CSS `width:100vw; height:100vh; object-fit:fill; user-select:none`）+ 拖拽层
- [ ] 拖拽：`onMouseDown` 记起点、`onMouseMove` 更新 `.shot-selection`（absolute，left/top/width/height，**物理像素 = CSS 像素 × devicePixelRatio 后取整**）、`onMouseUp` 判定 `w < 8 || h < 8` → `invoke("screenshot_cancel")`，否则 `invoke("screenshot_finish", { x, y, w, h })`
- [ ] 取消路径：`document` 级 `keydown`（Escape）与 `window` `blur` → `invoke("screenshot_cancel")`；`.shot-selection` CSS：`border: 1px solid #4f8cff; box-shadow: 0 0 0 100000px rgba(0,0,0,.35)`
- [ ] 验证：`npx tsc --noEmit`；`git commit -m "feat: 截图遮罩框选交互（拖选/ESC/失焦取消）"`

### Task 7: OCR 接入与原文卡片（M2）

**Files:** Modify `src-tauri/src/screenshot.rs`（finish 管线）、`src/App.tsx`（Popup 加截图模式）

- [ ] `screenshot.rs` 新命令 `pub fn copy_text(text: String) -> Result<(), String>`（arboard：`Clipboard::new()?.set_text(text)`），注册进 invoke_handler
- [ ] finish：hide screenshot 窗口 → `crop` → `encode_png` →（M1 调试：写 `%TEMP%\wordpeek-crop.png` 并 println 路径）→ `ocr::recognize(&png, &cfg.ocr_lang)`（`cfg` 取 `SharedConfig`）→ 成功：拼 `lines.map(|l| l.text).join("\n")`；emit_to `"main"` 事件 `screenshot-result` `{ ok: true, text, error: null }`；失败 `{ ok: false, text: null, error: <msg> }`；结束后若 `settings_was_visible` 则恢复 settings 显示
- [ ] Popup：`useState` 存截图结果卡片（`{ pending, text, error }`）；listen `screenshot-result`；渲染：原文卡片 + 「复制原文」按钮（`invoke("copy_text", { text })`）；划词 `selection` 事件到达时退出截图卡片视图

### Task 8: 翻译接入与设置项（M3/M4）

**Files:** Modify `src-tauri/src/screenshot.rs`、`src-tauri/src/main.rs`、`src/App.tsx`

- [ ] finish 中 OCR 成功后：`tauri::async_runtime::block_on(crate::translate::translate_text(&cfg, &text))`，`WordInfo` 序列化进 `screenshot-result` 载荷 `{ ok, text, info, error }`（M2 的 `info: null` 占位此时填实）
- [ ] 卡片双语化：senses 走 Popup 现有 `splitSense` 同款渲染，加「复制译文」（`info.senses.join("\n")`）
- [ ] 设置页 `SECTIONS` 加「截图翻译」：`screenshot_hotkey`（复用划词热键录制的同款交互，代码抽成 `HotkeyRecorder` 组件复用）与 `ocr_lang`（select：auto/中文/英文）；保存进 payload；TS `AppConfig` 补 `screenshotHotkey`、`ocrLang` 字段（字段名与后端 serde rename 保持一致）
- [ ] main.rs `register_hotkeys`：两个热键 `parse_hotkey` 成功且互不相同（`eq_ignore_ascii_case` 校验，相同返回「划词与截图快捷键不能相同」）才注册；单侧失败不阻断另一侧

### Task 9: README 与手动验证清单（收尾）

- [ ] README 增补：功能简介、快捷键表（划词 `Ctrl+Alt+T` / 截图 `Ctrl+Alt+S`）、OCR 语言设置说明、「仅 Windows、混合 DPI 多屏尽力而为」局限
- [ ] 手动验证：全屏遮罩出现且画面冻结 → ESC 取消；拖框 ≥8px → `%TEMP%\wordpeek-crop.png` 内容正确（M1 遗留调试可删）；OCR 中文/英文/中英混排；有道主源整句、百度未配置时降级；托盘「截图翻译」入口；设置修改两个快捷键并重启生效；失焦取消；125% 缩放单屏；多屏同 DPI
- [ ] 终检：`cd src-tauri && cargo test` 全绿 + `npx tsc --noEmit` 无错；`git commit -m "feat: 截图翻译完整流程与文档"`

---

## 附录 A：Task 1 执行进度与 windows 0.58 签名修正记录

> 2026-10-09 执行会话遗留：Task 1 代码已全部落盘，卡在最后 1 个编译错误。下个执行会话从「恢复步骤」继续，勿重写已有文件。

### 已完成（勿重复）
- `src-tauri/src/capture.rs` 已写全：`Rect`/`CapturedScreen` 类型、4 个单测、`crop`/`encode_png`/`capture_virtual_screen` 实现
- `main.rs` 已声明 `mod capture;`（`mod translate;` 之前）
- `Cargo.toml` 已加 `image 0.25（仅 png feature）`；windows features 已加 `Win32_Graphics_Gdi`

### windows 0.58 真实签名（与计划假设的差异，代码已按此修正）
| 计划假设 | 0.58 实际签名 | 已做的修正 |
|---|---|---|
| `BitBlt` 返回 BOOL | 返回 `windows_core::Result<()>` | 改 `if blit.is_err()` |
| `CreateCompatibleDC(Some(dc))` | 参数 `P0: Param<HDC>`，不接受 `Option` | 改 `CreateCompatibleDC(screen_dc)` |
| `BitBlt(..., Some(screen_dc), ...)` | `hdcsrc` 同样直收 `HDC` | 改 `BitBlt(mem_dc, 0, 0, w, h, screen_dc, ox, oy, SRCCOPY)` |
| `PngEncoder::encode(...)` | image 0.25 无此固有方法 | 改 `.write_image(...)` 并 `use image::ImageEncoder;` |
| `SelectObject(mem_dc, bitmap.into())` | `HBITMAP` 本身满足 `Param<HGDIOBJ>` | 去掉 `.into()`，直接 `SelectObject(mem_dc, bitmap)` |

### 恢复步骤
1. `cd src-tauri && cargo test capture` —— 预期仅剩 1 个错误：约 107 行残留 turbofish 调用
   `SelectObject::<HDC, _>(mem_dc, old);`（`HDC` 未导入 → E0425）。
   改为普通调用 `SelectObject(mem_dc, old);` —— `mem_dc: HDC`、`old: HGDIOBJ` 对两个泛型参数均唯一匹配，无需标注。
2. 若出现 `HGDIOBJ` unused import 警告，把它从 use 列表删除即可。
3. `cargo test capture` 全绿（4 passed）后，按 Task 1 Step 6 提交：
   `git add -A && git commit -m "feat: GDI 抓屏模块（虚拟屏截取/裁剪/PNG 编码）"`
4. 继续 Task 2。注意：OCR 相关 WinRT API（`BitmapDecoder::CreateAsync` 等）同样可能返回 `Result` 包装，以编译器提示为准，勿照抄计划中的 Option 假设。

---

## 附录 B：Task 2 完成记录

> 2026-10-09：Task 1、2 已完成并提交（`85a8867`、`a8e0e49`），全量 `cargo test` 12 passed 无回归。

### Task 2 关键修正（下个执行者注意）
- windows features 必须含 **`Foundation_Collections`**：`OcrEngine::AvailableRecognizerLanguages`（cfg: `Foundation_Collections` + `Globalization`）与 `OcrResult::Lines`（cfg: `Foundation_Collections`）被 feature 门控，缺了报 E0599
- `recognize` 其余 WinRT API（`DataWriter::CreateDataWriter`/`Seek`/`StoreAsync`/`BitmapDecoder::CreateAsync`/`RecognizeAsync` 等）在 0.58 均返回 `Result` 包装，按计划代码的 `.map_err(..)?` 链式写法一次编译通过
- `InMemoryRandomAccessStream` 的方法为 inherent，无需 cast 到接口

### 当前进度与恢复点
- ✅ Task 1（capture.rs）→ commit `85a8867`
- ✅ Task 2（ocr.rs）→ commit `a8e0e49`
- ⏭ **从 Task 3 开始**（translate.rs 重构：`translate_text` 抽取 + 有道整句分支，TDD：先写 `youdao_translates_full_sentence` 失败测试）

---

## 附录 C：Task 1 真机冒烟发现的两个 GDI 坑（已修复）

> 冒烟测试 `smoke_real_pipeline`（ignored，`cargo test smoke_real_pipeline -- --ignored --nocapture`）真机验证：4096x1440 多屏虚拟屏抓取成功、OCR 识别 34 行，抓屏→PNG→OCR 全链路打通。

1. **GetDIBits 时序坑**：MSDN 要求调用时位图不能被选入任何 DC —— `SelectObject(mem_dc, old)` 必须放在 `GetDIBits` **之前**，否则返回 0 行
2. **biSize 坑**：windows crate 的 `BITMAPINFOHEADER::default()` 是全零结构体，`biSize=0` 导致 GetDIBits 直接失败 —— 必须显式 `biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32`
3. 错误消息已区分 BitBlt / GetDIBits 两个失败点，便于定位

---

## 附录 D：Task 3 真机偏差——有道免费整句接口已下线（对 spec 的偏离）

- 计划假设 deskdict jsonapi 整句返回 `fanyi.tran`，真机实测无该节点（响应仅含 ee/blng_sents_part/input/meta/le/wikipedia_digest/lang）
- `fanyi.youdao.com/translate` 返回 HTTP 302（接口废弃）；`jsonversion=4` 需要签名。零 key 的有道整句通道已不可用
- **降级决策**：整句回落链 = 有道（保留 fanyi 节点兜底代码）→ 百度（需 key）。有道整句报错信息：「有道不支持整句翻译，请配置百度密钥后使用整句翻译」，`translate_text` 自动降级
- **对 spec 影响**：零配置仅覆盖单词查询；**整句翻译需要百度 key**。M3 截图翻译整句场景同此链路，错误信息会引导配置
- 后续若有稳定零 key 整句通道（如第三方代理或恢复的接口），替换 `translate_youdao` 整句分支即可
