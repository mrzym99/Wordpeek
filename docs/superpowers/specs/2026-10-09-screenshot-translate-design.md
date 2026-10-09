# 截图翻译 设计文档

- 日期：2026-10-09
- 状态：已与用户确认（OCR 用 Windows 系统自带，结果先做双语卡片、保留行坐标供日后原位覆盖）
- 相关里程碑：见文末

## 背景与目标

Wordpeek 目前只支持划词翻译（模拟 Ctrl+C 取选中文本）。截图翻译让用户按快捷键框选屏幕上任意区域，对区域内的文字做 OCR 识别并翻译，覆盖"文字无法选中"的场景（图片、PDF 影印件、部分应用 UI）。

**目标：**

1. 全局快捷键（默认 `Ctrl+Alt+S`）+ 托盘菜单项触发截图翻译
2. 全屏定格 → 拖拽框选 → Windows 系统 OCR 识别 → 复用现有翻译管线（有道/百度）→ 双语卡片
3. 零配置可用：OCR 免 Key、离线；翻译沿用现有配置
4. OCR 结果保留行级坐标，为将来"原位覆盖"展示铺路（本期不使用）

**非目标（本期不做）：**

- 原位覆盖渲染（译文画在屏幕原文字位置）
- 云端 OCR / 多模态大模型 OCR（接口留好，后续可加）
- 截图编辑、钉图、放大镜
- 非 Windows 平台（与现有项目一致，仅 Windows）

## 总体流程

```
Ctrl+Alt+S / 托盘「截图翻译」
  → Rust: BitBlt 抓取整个虚拟屏幕（多屏一张图，物理像素）
  → screenshot 窗口定位并铺满虚拟屏幕，emit "screenshot-start"
      携带全屏截图 data URL → 前端铺满窗口（画面定格，遮罩自身不入镜）
  → 用户拖拽框选（框内恢复全亮）/ ESC 取消 / 遮罩失焦取消 / 小于阈值视为点击取消
  → invoke("screenshot_finish", { x, y, w, h })   // 物理像素，相对虚拟屏幕
  → Rust: 隐藏遮罩 → 裁剪 → PNG 编码 → 后台线程：
      Windows OCR（文本 + 行坐标）→ translate_text()（有道→百度降级）
  → emit "screenshot-pending" { x, y }   // 立刻在选区旁显示「识别中…」卡片
  → emit "screenshot-result" { ok, original, info?, error? }
  → 复用 main 弹窗（放大尺寸）显示双语卡片
```

要点：

- **截图先于遮罩出现**，框选的是静止画面，遮罩不会被截进去
- **复用 main 弹窗**展示结果，不新增第四个常驻窗口
- OCR 与翻译在后台线程串行执行，期间屏幕已恢复正常

## 模块设计

### `src-tauri/src/capture.rs`（新增）

GDI 抓屏，仅 Windows 编译。

```rust
pub struct CapturedScreen {
    pub width: i32,     // 虚拟屏幕宽（物理像素）
    pub height: i32,
    pub origin_x: i32,  // 虚拟屏幕原点（多屏时可能为负）
    pub origin_y: i32,
    pub rgba: Vec<u8>,  // 32bpp RGBA，按行排列
}

pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

pub fn capture_virtual_screen() -> Result<CapturedScreen, String>
pub fn crop(rgba: &[u8], width: i32, height: i32, rect: Rect) -> Result<Vec<u8>, String>  // 纯函数，单测
pub fn encode_png(rgba: &[u8], width: i32, height: i32) -> Result<Vec<u8>, String>
```

实现要点：

- `GetSystemMetrics(SM_XVIRTUALSCREEN / SM_YVIRTUALSCREEN / SM_CXVIRTUALSCREEN / SM_CYVIRTUALSCREEN)` 定位虚拟屏幕
- `CreateCompatibleDC` + `CreateCompatibleBitmap` + `BitBlt(SRCCOPY)` + `GetDIBits`（负高度取 top-down，BI_RGB 32bpp）
- GDI 句柄逐一清理（ReleaseDC / DeleteDC / DeleteObject）
- GetDIBits 返回 BGRA：交换 B/R 通道、alpha 置 255
- PNG 编码用 `image` crate（`default-features = false, features = ["png"]`）

### `src-tauri/src/ocr.rs`（新增）

Windows.Media.Ocr 封装，仅 Windows 编译。

```rust
pub struct OcrLine {
    pub text: String,
    pub x: i32, pub y: i32, pub width: i32, pub height: i32, // 相对裁剪区，物理像素
}

/// lang: "auto" | "zh" | "en"
pub fn recognize(png: &[u8], lang: &str) -> Result<Vec<OcrLine>, String>
```

实现要点：

- 初始化 WinRT apartment（MTA）后调用（`Win32_System_WinRT` / `RoInitialize`）
- PNG 字节 → `InMemoryRandomAccessStream` + `DataWriter` → `BitmapDecoder` → `GetSoftwareBitmapAsync`（Bgra8）→ `OcrEngine::RecognizeAsync`（`.get()` 阻塞等待，调用方已在线程池线程）
- 引擎创建：`auto` → `TryCreateFromUserProfileLanguages`；`zh` / `en` → 从 `AvailableRecognizerLanguages` 按标签前缀匹配（`zh` 优先 `zh-Hans`，`en` 优先 `en-US`），再 `TryCreateFromLanguage`
- 找不到可用语言包 → 返回友好错误：「系统未安装中/英文 OCR 语言包，请在 Windows 设置 → 时间和语言 → 语言和区域 中添加」
- 行包围盒：对每行 `OcrWord.BoundingRect` 取并集（浮点坐标四舍五入）
- `windows` crate 增开 features：`Win32_Graphics_Gdi`、`Media_Ocr`、`Globalization`、`Graphics_Imaging`、`Storage_Streams`、`Foundation`、`Win32_System_WinRT`

### `src-tauri/src/screenshot.rs`（新增）

流程编排与状态：

```rust
pub struct ScreenshotState {
    full: Option<CapturedScreen>,        // 本次定格的全屏图
    settings_was_visible: bool,          // 触发时设置窗口是否可见（结束后恢复）
}
```

- `handle_screenshot_trigger(app)`（快捷键 / 托盘共用）：隐藏 main 弹窗（若可见）与 settings（若可见并记录）→ 抓屏 → 存入 `ScreenshotState` → screenshot 窗口 `set_position/set_size`（虚拟屏幕原点与尺寸，物理像素）→ show + set_focus → emit `screenshot-start { data_url, width, height }`
- 命令 `screenshot_finish(x, y, w, h)`：隐藏遮罩 → 恢复设置窗口可见性 → 后台线程：`crop` → `encode_png` → emit `screenshot-pending`（x/y = 选区左上角屏幕坐标，Rust 端按卡片尺寸钳制）→ `ocr::recognize` → `translate::translate_text` → emit `screenshot-result`；任一步失败 emit 带 `error` 的结果
- 命令 `screenshot_cancel`：隐藏遮罩、恢复设置窗口可见性、清空 state
- 命令 `copy_text(text)`：用 arboard 写剪贴板（卡片「复制译文」用，webview 剪贴板 API 不可靠）
- 抓屏失败：记日志、静默返回（等价于没按）

### `src-tauri/src/translate.rs`（改动）

- 抽出内部函数 `pub async fn translate_text(cfg: &AppConfig, text: &str) -> Result<WordInfo, String>`，现有 `translate` 命令改为薄封装（读 State 后调用）。screenshot 流程直接用它，避免依赖 `State`
- **有道源补整句翻译分支**：`ec.word[0]` 为空时回落到响应 `fanyi.tran` 节点，返回 `WordInfo { word: 查询文本, senses: [tran], source: "youdao" }`；两者皆空才报「未找到释义」。此修复同时让现有划词支持整句
- OCR 文本整体作为一次翻译请求（不做分行请求，YAGNI；换行符原样传递）

### `src-tauri/src/main.rs`（改动）

- `register_hotkey` → `register_hotkeys`：读配置里的 `hotkey` + `screenshot_hotkey`，`unregister_all` 后逐一注册；两者字符串相同 → 报错；某个注册失败不阻断另一个，错误信息合并返回（标明是哪个快捷键）
- `save_config` 保存后调用 `register_hotkeys`
- 托盘菜单加「截图翻译」项 → `screenshot::handle_screenshot_trigger`

### 前端（`src/`）

**`ScreenshotPage.tsx`（新增，`?page=screenshot` 路由）：**

- 监听 `screenshot-start`，`<img>` 全屏铺满冻结截图，上面盖半透明黑蒙层
- 拖拽画选框：框内透出原图（box-shadow 大 spread 实现框外变暗），边框高亮，右下角显示 `宽×高`
- 松开鼠标：区域 < 8×8 物理像素视为点击 → `screenshot_cancel`；否则 CSS 坐标 × `devicePixelRatio`（`window.scaleFactor()`）取整后 `screenshot_finish`
- `keydown Escape` / `tauri://blur` → `screenshot_cancel`

**`Popup`（改造）：**

- 新增监听 `screenshot-pending { x, y }`：`setSize(480×460 物理像素)` → `setPosition` → show → focus → 显示「识别中…」
- 新增监听 `screenshot-result`：渲染截图卡片——头部标题 + 原文区（可滚动、弱化样式）+ 译文区（复用现有 senses 列表渲染）+「复制译文」按钮（`copy_text`）
- 词典卡片逻辑（音标/词形/同义词）保持不变；两种模式互斥，互不干扰

**`SettingsPage`（改造）：**

- `SECTIONS` 增加「截图翻译」分组：截图翻译快捷键（可录制）+ OCR 识别语言下拉（跟随系统 / 中文 / English）
- 录制逻辑抽成 `HotkeyRecorder` 小组件，划词与截图两处复用
- `save` 提交完整配置（含新字段）；`screenshot_hotkey` 与 `hotkey` 相同时前端提前报错提示

### 配置（`translate.rs::AppConfig` 新增字段）

```rust
#[serde(default = "default_screenshot_hotkey")]
pub screenshot_hotkey: String,   // 默认 "Ctrl+Alt+S"
#[serde(default)]
pub ocr_lang: String,            // "auto" | "zh" | "en"，默认 "auto"
```

旧配置文件无这两个字段 → serde default 补齐，向后兼容。

## 配置与权限

- `tauri.conf.json`：`windows` 数组新增 screenshot 窗口——`decorations: false, transparent: true, alwaysOnTop: true, skipTaskbar: true, visible: false, resizable: false, shadow: false`，初始尺寸随意（Rust 显示前会重设）
- `tauri.conf.json` CSP：`default-src` 基础上追加 `img-src 'self' data:`
- `capabilities/default.json`：`windows` 数组加 `"screenshot"`（权限集沿用现有）

## 错误处理

| 场景 | 表现 |
|---|---|
| 抓屏失败 | eprintln 日志，静默返回 |
| 系统缺 OCR 语言包 | 卡片内提示到 Windows 设置添加语言包 |
| OCR 无文字 | 卡片显示「未识别到文字」 |
| 翻译失败（含降级均失败） | 卡片显示错误（复用现有 error 展示） |
| ESC / 遮罩失焦 / 点击未拖拽 | 取消，不产生结果，恢复现场 |
| 截图快捷键注册失败（被占用） | 启动时日志告警，划词快捷键不受影响 |

## 取舍与已知限制

- **混合 DPI 多显示器**：GDI 抓虚拟屏幕在主副屏 DPI 不同时，副屏内容可能缩放异常。按"尽力而为"处理：主屏与同 DPI 多屏完全正常；README 标注
- **全屏截图 data URL 经 IPC 传输**：2K/4K 全屏 PNG base64 约几 MB，一次性传输可接受；若实测卡顿，备选方案是自定义 `screenshot://` URI 协议，本期不做
- **OCR 语言包**：中文识别依赖系统已装中文语言包（Win10/11 中文系统默认有）
- **选区即整段翻译**：不区分块逐块翻译，长文本一次请求（百度 API 有长度上限，超长会报 API 错误，卡片如实展示）

## 测试策略

**Rust 单测（`cargo test`）：**

- `capture::crop`：正常裁剪、越界钳制、坐标换算
- 配置默认值：无字段时 `screenshot_hotkey`/`ocr_lang` 正确补齐
- `register_hotkeys` 输入校验：两快捷键相同报错
- 有道整句分支：真实网络请求断言 `Ok` 且 senses 非空（沿用现有测试风格）

**OCR 不做自动化单测**（依赖系统语言包与图形环境），转手动验证。

**手动验证清单（每里程碑收尾过一遍）：**

1. 单屏框选 → 卡片在选区旁弹出，内容正确
2. 多屏（同 DPI）在副屏截图 → 坐标正确
3. 125% DPI 下框选 → 裁剪区域与所见一致
4. ESC / 失焦 / 纯点击 → 正常取消，无残留窗口
5. 托盘「截图翻译」触发 → 与快捷键行为一致
6. 有道源与百度源分别出结果；断网时错误文案可读
7. 截图过程中设置窗口原可见 → 结束后恢复可见
8. 中英混排截图识别率可接受

## 里程碑

- **M1 截屏 + 框选闭环**：capture.rs、screenshot 窗口与 ScreenshotPage 交互、finish/cancel 命令；先只打印裁剪尺寸（写临时 PNG 供肉眼验证）
- **M2 接入 OCR**：ocr.rs、后台管线、pending/result 事件、卡片显示识别原文（未接翻译）
- **M3 接入翻译**：translate_text 抽取、有道整句分支、双语卡片 + 复制按钮
- **M4 设置与打磨**：设置新分组、双快捷键注册与冲突校验、托盘菜单项、失焦/恢复细节、README 更新
