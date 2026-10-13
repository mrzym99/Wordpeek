# OCR 识别准确率优化 实施计划（issue #2 第 3 条）

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 提升截图 OCR 在「背景复杂」「多种字体混排」场景下的识别率：放大更狠、深底反色、自适应二值化、多语言引擎择优，并给出一套可量化的改造前后对照方法。

**Architecture:** 把 `ocr.rs` 拆成两半——跨平台纯函数区（放大倍数、分位拉伸、自适应二值化、文本质量打分、引擎候选与尝试编排，全部可单测）+ Windows 执行区（用 `windows` crate 跑 `Windows.Media.Ocr`）。`recognize(png, lang)` 对外签名不变：内部先做图像预处理得到两个变体（分位拉伸 / 局部自适应二值化），再按「主引擎×两变体 → 备用语言引擎」的预算内顺序尝试，用文本质量打分挑最好的结果返回。

**Tech Stack:** Rust、`image` 0.25（仅 png，无需新增特性）、`windows` 0.58（`Media_Ocr` / `Graphics_Imaging`，已在依赖里）。

**Spec:** `https://github.com/mrzym99/Wordpeek/issues/2` 第 3 条；设计决策与人工验收协议见本文「设计决策」「Task 4」。

## 设计决策

| 决策 | 选择 | 理由 / 否决的替代方案 |
|---|---|---|
| 引擎 | 继续用 Windows 系统 OCR，只在管线上下功夫 | 保住「免 Key、离线、数据不出本机」的承诺；用户已明确**不接受安装包变大**，因此不考虑随包 ONNX 模型（PP-OCR 之类体积 +20MB 以上）。管线调优若仍不达标，结论按「当前引擎的能力边界」处理并在 README 如实标注 |
| 放大 | 短边目标 150px → **300px**，倍数 1~4，且最长边 ≤ 4000px | 小字号是 Windows OCR 的头号失分点（现有 150px 阈值太低）；4000px 远低于引擎的 `MaxImageDimension`（约 10000），避免超大图把识别拖成秒级 |
| 反色 | 均值 < 110 时反色 | 引擎对「浅底深字」明显更好；深色主题截图/深底代码块是常见场景 |
| 二值化 | 局部自适应（积分图，窗口约短边/32，偏置 8）+ 高斯模糊 σ=0.8 去噪 | 全局阈值会被背景渐变带跑；自适应二值化专治「背景稍微复杂」。保留一个「只做分位拉伸」的变体，因为二值化会削掉抗锯齿，某些字体反而变差——两个都试、用打分择优。已知取舍：窗口只有十几到几十像素，当笔画远粗于窗口（字号极大、实心色块字）时局部均值会被笔画本身带跑（与 OpenCV `adaptiveThreshold` 同类行为）；OCR 场景的文字是笔画式的，且拉伸变体兜底 |
| 多语言 | 显式 `zh`/`en` 时另一种语言作备用引擎；`auto` 时跟随系统 + 可用中/英文备用 | 中英混排、外语异体字场景换引擎常常直接改善；备用引擎只在主引擎打分偏低（< 0.8）时才跑，常见情况下不多花时间 |
| 结果择优 | 文本质量打分 `(字母/数字数 − 2×其它非空白字符数)/字符数`，取最高分 | Windows OCR 不暴露置信度（已核实 0.58 绑定里 `OcrLine`/`OcrWord` 只有文本与包围盒）；用「像不像正常文本」做代理指标，纯函数可测 |
| 尝试预算 | 最多 4 次识别（主引擎×2 变体 + 备用引擎×1~2），得分 ≥ 0.8 立即早退 | OCR 单次 ~100~400ms，必须封顶；早退保证常见场景只跑 1 次 |
| 像素格式 | 解码后统一 `SoftwareBitmap::ConvertWithAlpha(Bgra8, Premultiplied)` | 现在传的是 RGBA PNG（能跑通），但预处理后会产出灰度 PNG——显式转 Bgra8 才能保证引擎一定接受 |
| 坐标契约 | OCR 坐标除以放大倍数还原回裁剪图坐标系 | 现存文档承诺「坐标相对裁剪区」，而放大后返回的是放大图坐标（一直没被使用所以没人发现）；顺手补齐，别再留坑 |

## Global Constraints

- 不新增依赖；`windows` crate 已有 features 足够（`Media_Ocr`、`Graphics_Imaging`、`Storage_Streams`、`Foundation`、`Foundation_Collections`、`Globalization`）
- 对外契约不变：`ocr::recognize(png: &[u8], lang: &str) -> Result<Vec<OcrLine>, String>`、`OcrLine { text, x, y, w, h }`、`cts_lang`（`"auto" | "zh" | "en"`）与 `main.rs` 的调用点都不改
- 错误语义不变：系统缺语言包仍要返回「请在 Windows 设置 → 时间和语言 → 语言和区域 中添加」这段可读文案
- 跨平台可编译：新增纯函数一律不加 `#[cfg(windows)]`；仅执行区代码门控
- 注释与用户可见文案一律中文；禁止 TODO/TBD
- 每个 Rust 任务收尾必须 `cd src-tauri && cargo test` 通过
- 提交信息中文，格式 `feat|fix|test|docs: 描述`

---

### Task 1: 图像预处理纯函数（放大 / 反色 / 分位拉伸 / 自适应二值化 / 打分）

**Files:**
- Modify: `src-tauri/src/ocr.rs`（重写文件上半部分为纯函数区；删除已被吸收的 `upscale_if_small`）
- Test: `src-tauri/src/ocr.rs` 的 `mod tests`

**Interfaces:**
- Consumes: `image` crate（`DynamicImage::to_luma8`、`imageops::{resize, blur, invert}`、`FilterType::Lanczos3`、`GrayImage`、`ImageBuffer::from_vec`）
- Produces:
  - `pub(crate) enum PrepMode { Stretch, Binarize }`（`name()` 返回 `"stretch"` / `"binarize"`）
  - `pub(crate) struct Prep { pub mode: PrepMode, pub png: Vec<u8> }`、`pub(crate) struct Prepped { pub variants: Vec<Prep>, pub scale: u32 }`
  - `pub(crate) fn upscale_factor(w: u32, h: u32) -> u32`
  - `pub(crate) fn pixel_mean(img: &image::GrayImage) -> f32`
  - `pub(crate) fn should_invert(mean: f32) -> bool`
  - `pub(crate) fn percentile_bounds(hist: &[u32; 256], low_pct: f32, high_pct: f32) -> (u8, u8)`
  - `pub(crate) fn stretch_contrast(img: &image::GrayImage) -> image::GrayImage`
  - `pub(crate) fn adaptive_binarize(img: &image::GrayImage, c: i32) -> image::GrayImage`
  - `pub(crate) fn score_text(s: &str) -> f64`
  - `pub(crate) fn scale_lines(lines: &mut [OcrLine], f: u32)`
  - `pub(crate) fn preprocess(png: &[u8]) -> Result<Prepped, String>`（含 `WORDPEEK_OCR_RAW=1` 时跳过全部预处理，仅用于 A/B 对照）

- [ ] **Step 1: 写失败测试**

把 `ocr.rs` 的 `mod tests` 中 `upscale_factor_reaches_min_side` 与 `upscale_if_small_enlarges_image` 两个测试**整段替换**为下面这些（`upscale_if_small` 已被 `preprocess` 吸收）：

```rust
    #[test]
    fn upscale_factor_reaches_min_side_without_blowing_up() {
        assert_eq!(upscale_factor(50, 20), 4); // 150/20=7.5 → 封顶 4
        assert_eq!(upscale_factor(120, 80), 4); // 300/80=3.75 → 4
        assert_eq!(upscale_factor(800, 400), 1); // 已够大
        assert_eq!(upscale_factor(2000, 40), 2); // 长边已 2000，最多再 ×2
        assert_eq!(upscale_factor(6000, 300), 1); // 长边超 4000，不再放大
    }

    #[test]
    fn percentile_bounds_ignores_outliers() {
        let mut hist = [0u32; 256];
        hist[20] = 100;
        hist[200] = 100;
        assert_eq!(percentile_bounds(&hist, 2.0, 98.0), (20, 200));
        // 单色图不会算出 lo == hi（否则映射要除零）
        let mut flat = [0u32; 256];
        flat[128] = 50;
        assert_eq!(percentile_bounds(&flat, 2.0, 98.0), (128, 129));
        assert_eq!(percentile_bounds(&[0u32; 256], 2.0, 98.0), (0, 255));
    }

    #[test]
    fn stretch_contrast_maps_dark_and_light_to_ends() {
        let mut img = image::GrayImage::new(4, 1);
        img.put_pixel(0, 0, image::Luma([100]));
        img.put_pixel(1, 0, image::Luma([100]));
        img.put_pixel(2, 0, image::Luma([200]));
        img.put_pixel(3, 0, image::Luma([200]));
        let out = stretch_contrast(&img);
        assert_eq!(out.get_pixel(0, 0)[0], 0);
        assert_eq!(out.get_pixel(3, 0)[0], 255);
    }

    #[test]
    fn adaptive_binarize_separates_text_from_shaded_background() {
        // 背景亮度 60 → 200 的横向渐变，中间一块比背景暗 50 的"文字"
        let (w, h) = (80u32, 40u32);
        let mut img = image::GrayImage::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let bg = 60 + (140 * x / w) as u8;
                let in_text = (20..60).contains(&x) && (15..25).contains(&y);
                let v = if in_text { bg.saturating_sub(50) } else { bg };
                img.put_pixel(x, y, image::Luma([v]));
            }
        }
        let out = adaptive_binarize(&img, 8);
        assert_eq!(out.get_pixel(40, 20)[0], 0); // 文字块内是黑
        assert_eq!(out.get_pixel(5, 5)[0], 255); // 渐变背景是白
        assert_eq!(out.get_pixel(75, 35)[0], 255);
    }

    #[test]
    fn score_text_prefers_real_words() {
        assert!((score_text("hello world") - 1.0).abs() < 1e-9);
        assert!(score_text("@@@@@") < 0.0);
        assert_eq!(score_text("   "), 0.0);
        let cn = score_text("你好，世界");
        assert!(cn > 0.0 && cn < 1.0, "中文标点只扣分不归零: {cn}");
        assert!(score_text("Hello world") > score_text("H3||o w~rld"));
    }

    #[test]
    fn scale_lines_restores_crop_coordinates() {
        let mut lines = vec![OcrLine {
            text: "x".into(),
            x: 100,
            y: 50,
            w: 40,
            h: 20,
        }];
        scale_lines(&mut lines, 4);
        assert_eq!((lines[0].x, lines[0].y, lines[0].w, lines[0].h), (25, 13, 10, 5));
        // 未放大时不改坐标
        let mut same = lines.clone();
        scale_lines(&mut same, 1);
        assert_eq!((same[0].x, same[0].y), (25, 13));
    }

    #[test]
    fn should_invert_only_for_dark_backgrounds() {
        assert!(should_invert(40.0));
        assert!(!should_invert(200.0));
        assert!(!should_invert(110.0));
    }

    #[cfg(windows)]
    #[test]
    fn preprocess_yields_two_variants_and_scales_small_crops() {
        use crate::capture::encode_png;
        // 50×20 的小图：应先放大 4 倍，再产出两个变体
        let rgba = vec![255u8; 50 * 20 * 4];
        let png = encode_png(&rgba, 50, 20).expect("编码应成功");
        let p = preprocess(&png).expect("预处理应成功");
        assert_eq!(p.scale, 4);
        assert_eq!(p.variants.len(), 2);
        for v in &p.variants {
            let img = image::load_from_memory(&v.png).expect("变体应是合法 PNG");
            assert_eq!((img.width(), img.height()), (200, 80));
        }
        // 已够大的图不放大，但仍有二值化变体
        let big = encode_png(&vec![255u8; 800 * 400 * 4], 800, 400).expect("编码应成功");
        assert_eq!(preprocess(&big).expect("预处理应成功").scale, 1);
    }

    #[cfg(windows)]
    #[test]
    fn preprocess_inverts_dark_backgrounds_and_binarizes() {
        // 深底（20）浅字（230）：用 800×400 让缩放为 1；竖条宽 12px、间隔 40px
        // （窗口半径 = 短边/32 = 12px，笔画比窗口窄，局部均值才有对比度）
        let (w, h) = (800u32, 400u32);
        let mut img = image::GrayImage::from_pixel(w, h, image::Luma([20]));
        for x in (40..760).step_by(40) {
            for y in 40..360 {
                for dx in 0..12 {
                    img.put_pixel(x + dx, y, image::Luma([230]));
                }
            }
        }
        let p = preprocess(&encode_gray(&img).expect("编码应成功")).expect("预处理应成功");
        assert_eq!(p.scale, 1);

        // 拉伸变体：反色生效 → 笔画变深、背景变亮
        let stretch = image::load_from_memory(&p.variants[0].png)
            .unwrap()
            .to_luma8();
        assert!(stretch.get_pixel(46, 200)[0] < 64, "反色后笔画应偏黑");
        assert!(stretch.get_pixel(70, 200)[0] > 192, "反色后背景应偏白");

        // 二值化变体：笔画为黑、笔画间隔为白
        let bin = image::load_from_memory(&p.variants[1].png)
            .unwrap()
            .to_luma8();
        assert_eq!(bin.get_pixel(46, 200)[0], 0, "笔画应为黑");
        assert_eq!(bin.get_pixel(70, 200)[0], 255, "笔画间隔应为白");
    }
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cd src-tauri && cargo test ocr::tests`

Expected: 编译失败（`upscale_factor` 参数/语义、`stretch_contrast`、`adaptive_binarize`、`score_text`、`scale_lines`、`preprocess` 都还不存在）

- [ ] **Step 3: 实现纯函数区**

把 `ocr.rs` 里 `cfg(windows)` 的 `upscale_if_small` 与旧的 `upscale_factor` **整段删除**，替换为下面内容（放在 `pick_language` 之后、`recognize` 之前）。

同时把旧 `recognize` 开头的这一行

```rust
    // 小图先放大再识别，否则 Windows OCR 对小字号基本识别不出
    let png = upscale_if_small(png)?;
```

换成临时桥接（Task 2 会把整个 `recognize` 重写成 `recognize_report` 版本，这里只是为了每个任务结束时代码仍可编译）：

```rust
    // 临时桥接：只剩分位拉伸变体，Task 2 换成完整的变体择优
    let png = preprocess(png)?.variants.remove(0).png;
```

```rust
/// OCR 前的图像处理方式
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PrepMode {
    /// 分位拉伸：保留灰度层次，适合浅底深字、背景明暗不均
    Stretch,
    /// 局部自适应二值化：压掉复杂纹理与渐变背景，适合照片式/彩色背景
    Binarize,
}

impl PrepMode {
    pub(crate) fn name(self) -> &'static str {
        match self {
            PrepMode::Stretch => "stretch",
            PrepMode::Binarize => "binarize",
        }
    }
}

/// 一个可直接送 OCR 的图像变体
#[derive(Debug, Clone)]
pub(crate) struct Prep {
    pub mode: PrepMode,
    pub png: Vec<u8>,
}

/// 预处理结果：变体 + 相对原裁剪图的放大倍数（用于把 OCR 坐标还原回去）
pub(crate) struct Prepped {
    pub variants: Vec<Prep>,
    pub scale: u32,
}

/// 短边放大到至少 300px（Windows OCR 对小字号识别极差），倍数 1~4，
/// 且放大后最长边不超过 4000px（远低于引擎上限，避免超大图把识别拖慢）。
pub(crate) fn upscale_factor(w: u32, h: u32) -> u32 {
    if w == 0 || h == 0 {
        return 1;
    }
    let by_short = 300.0 / w.min(h) as f32;
    let by_long = 4000.0 / w.max(h) as f32;
    by_short.min(by_long).ceil().clamp(1.0, 4.0) as u32
}

/// 灰度均值（反色判断用）
pub(crate) fn pixel_mean(img: &image::GrayImage) -> f32 {
    let raw = img.as_raw();
    if raw.is_empty() {
        return 0.0;
    }
    raw.iter().map(|v| *v as f32).sum::<f32>() / raw.len() as f32
}

/// 深色背景才反色：引擎更擅长「浅底深字」
pub(crate) fn should_invert(mean: f32) -> bool {
    mean < 110.0
}

/// 灰度直方图的 low%/high% 分位点：抗离群点，比 min/max 稳
pub(crate) fn percentile_bounds(hist: &[u32; 256], low_pct: f32, high_pct: f32) -> (u8, u8) {
    let total: u64 = hist.iter().map(|c| *c as u64).sum();
    if total == 0 {
        return (0, 255);
    }
    let lo_target = (total as f32 * low_pct / 100.0) as u64;
    let hi_target = (total as f32 * high_pct / 100.0) as u64;
    let mut acc = 0u64;
    let mut lo = 0u8;
    let mut hi = 255u8;
    let mut lo_done = false;
    for v in 0..256u32 {
        acc += hist[v as usize] as u64;
        // `.max(1)`：小图（几百像素）时 lo_target 会算成 0，
        // 不兜底会让 lo 直接落在 0，灰度被整体压暗
        if !lo_done && acc >= lo_target.max(1) {
            lo = v as u8;
            lo_done = true;
        }
        if acc >= hi_target {
            hi = v as u8;
            break;
        }
    }
    // 单色图：lo == hi 会让映射除零，抬一格兜底
    if hi <= lo {
        hi = lo.saturating_add(1);
    }
    (lo, hi)
}

/// 按 2%/98% 分位把灰度拉满到 0~255
pub(crate) fn stretch_contrast(img: &image::GrayImage) -> image::GrayImage {
    let raw = img.as_raw();
    let mut hist = [0u32; 256];
    for v in raw {
        hist[*v as usize] += 1;
    }
    let (lo, hi) = percentile_bounds(&hist, 2.0, 98.0);
    let (lo, hi) = (lo as f32, hi as f32);
    let span = (hi - lo).max(1.0);
    let out: Vec<u8> = raw
        .iter()
        .map(|v| (((*v as f32 - lo) / span) * 255.0).clamp(0.0, 255.0) as u8)
        .collect();
    image::GrayImage::from_vec(img.width(), img.height(), out).expect("尺寸固定，from_vec 不会失败")
}

/// 局部自适应二值化：阈值取窗口均值减去偏置 c，用积分图 O(1) 取窗口和，
/// 对背景明暗渐变、纸张纹理、照片底色都稳。输出纯黑白：文字 0 / 背景 255。
pub(crate) fn adaptive_binarize(img: &image::GrayImage, c: i32) -> image::GrayImage {
    let (w, h) = (img.width() as usize, img.height() as usize);
    let src = img.as_raw();
    // 积分图带一圈 0 边界：int[y][x] = 左上角 y×x 区域的和
    let mut integral = vec![0u64; (w + 1) * (h + 1)];
    for y in 0..h {
        let mut row_sum = 0u64;
        for x in 0..w {
            row_sum += src[y * w + x] as u64;
            integral[(y + 1) * (w + 1) + x + 1] = integral[y * (w + 1) + x + 1] + row_sum;
        }
    }
    let r = (w.min(h) / 32).clamp(7, 40);
    let mut out = vec![255u8; w * h];
    for y in 0..h {
        let y0 = y.saturating_sub(r);
        let y1 = (y + r + 1).min(h);
        for x in 0..w {
            let x0 = x.saturating_sub(r);
            let x1 = (x + r + 1).min(w);
            let area = ((x1 - x0) * (y1 - y0)) as u64;
            let sum = integral[y1 * (w + 1) + x1] + integral[y0 * (w + 1) + x0]
                - integral[y0 * (w + 1) + x1]
                - integral[y1 * (w + 1) + x0];
            if (src[y * w + x] as i32) < (sum / area) as i32 - c {
                out[y * w + x] = 0;
            }
        }
    }
    image::GrayImage::from_vec(img.width(), img.height(), out).expect("尺寸固定，from_vec 不会失败")
}

/// 文本质量打分（越高越像正常文本）：字母/数字 +1、其它非空白字符 −2，按字符数归一。
/// Windows OCR 不给置信度，用「像不像正常文本」当代理指标挑选引擎与预处理组合。
pub(crate) fn score_text(s: &str) -> f64 {
    let mut good = 0i32;
    let mut bad = 0i32;
    let mut n = 0i32;
    for ch in s.chars() {
        if ch.is_whitespace() {
            continue;
        }
        n += 1;
        if ch.is_alphanumeric() {
            good += 1;
        } else {
            bad += 1;
        }
    }
    if n == 0 {
        return 0.0;
    }
    (good - 2 * bad) as f64 / n as f64
}

/// OCR 坐标从放大图还原回原裁剪图
pub(crate) fn scale_lines(lines: &mut [OcrLine], f: u32) {
    if f <= 1 {
        return;
    }
    let d = f as f32;
    for l in lines.iter_mut() {
        l.x = (l.x as f32 / d).round() as i32;
        l.y = (l.y as f32 / d).round() as i32;
        l.w = (l.w as f32 / d).round().max(1.0) as i32;
        l.h = (l.h as f32 / d).round().max(1.0) as i32;
    }
}

/// 灰度图编码成 PNG（写回给 Windows 解码器）
fn encode_gray(img: &image::GrayImage) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    image::DynamicImage::ImageLuma8(img.clone())
        .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
        .map_err(|e| format!("PNG 编码失败: {e}"))?;
    Ok(out)
}

/// 生成 OCR 用的图像变体：
/// 1) 小图放大（小字号是最大失分点）；2) 深底浅字反色；3) 分别做分位拉伸与自适应二值化。
/// 设 `WORDPEEK_OCR_RAW=1` 时跳过全部预处理，仅用于人工 A/B 对照。
pub(crate) fn preprocess(png: &[u8]) -> Result<Prepped, String> {
    if std::env::var_os("WORDPEEK_OCR_RAW").is_some() {
        return Ok(Prepped {
            variants: vec![Prep {
                mode: PrepMode::Stretch,
                png: png.to_vec(),
            }],
            scale: 1,
        });
    }
    let img = image::load_from_memory(png).map_err(|e| format!("PNG 解码失败: {e}"))?;
    let gray = img.to_luma8();
    let scale = upscale_factor(gray.width(), gray.height());
    let scaled: image::GrayImage = if scale > 1 {
        image::imageops::resize(
            &gray,
            gray.width() * scale,
            gray.height() * scale,
            // Lanczos3：文字边缘保留最好，也最利于后续二值化
            image::imageops::FilterType::Lanczos3,
        )
    } else {
        gray
    };

    let mut base = scaled;
    if should_invert(pixel_mean(&base)) {
        image::imageops::invert(&mut base);
    }

    // 二值化前轻度高斯模糊：压掉 JPEG 噪点与背景纹理，同时保留笔画
    let denoised = image::imageops::blur(&base, 0.8);
    let variants = vec![
        Prep {
            mode: PrepMode::Stretch,
            png: encode_gray(&stretch_contrast(&base))?,
        },
        Prep {
            mode: PrepMode::Binarize,
            png: encode_gray(&adaptive_binarize(&denoised, 8))?,
        },
    ];
    Ok(Prepped { variants, scale })
}
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cd src-tauri && cargo test ocr::tests`

Expected: 全部 PASS（`preprocess_yields_two_variants_and_scales_small_crops` 仅 Windows 编译）

- [ ] **Step 5: 提交**

```bash
git add src-tauri/src/ocr.rs
git commit -m "feat: OCR 图像预处理（放大/反色/分位拉伸/自适应二值化）与结果打分"
```

---

### Task 2: 引擎候选、尝试编排与多引擎择优执行

**Files:**
- Modify: `src-tauri/src/ocr.rs`
- Test: `src-tauri/src/ocr.rs` 的 `mod tests`

**Interfaces:**
- Consumes: Task 1 的 `Prepped`/`Prep`/`preprocess`/`score_text`/`scale_lines`；现有 `pick_language`
- Produces:
  - `pub(crate) enum EngineSpec { UserProfile, Tag(String) }`（`name()` 返回可读名）
  - `pub(crate) fn candidate_engines(lang: &str, available: &[String]) -> Result<Vec<EngineSpec>, String>`
  - `pub(crate) fn attempt_plan(engines: &[EngineSpec]) -> Vec<(usize, PrepMode)>`
  - `pub(crate) const GOOD_SCORE: f64 = 0.8;`、`pub(crate) const MAX_ATTEMPTS: usize = 4;`
  - `pub fn recognize(png: &[u8], lang: &str) -> Result<Vec<OcrLine>, String>`（**签名不变**）
  - `#[cfg(windows)] pub(crate) struct OcrReport { pub lines: Vec<OcrLine>, pub score: f64, pub engine: String, pub mode: &'static str, pub attempts: usize }`
  - `#[cfg(windows)] pub(crate) fn recognize_report(png: &[u8], lang: &str) -> Result<OcrReport, String>`

- [ ] **Step 1: 写失败测试**

`mod tests` 追加：

```rust
    #[test]
    fn candidate_engines_primary_then_alternates() {
        let avail = vec!["en-US".to_string(), "zh-Hans".to_string()];
        assert_eq!(
            candidate_engines("zh", &avail).unwrap(),
            vec![
                EngineSpec::Tag("zh-Hans".into()),
                EngineSpec::Tag("en-US".into())
            ]
        );
        assert_eq!(
            candidate_engines("en", &avail).unwrap(),
            vec![
                EngineSpec::Tag("en-US".into()),
                EngineSpec::Tag("zh-Hans".into())
            ]
        );
        // auto：跟随系统在前，其后是可用的中英文备用引擎
        assert_eq!(
            candidate_engines("auto", &avail).unwrap(),
            vec![
                EngineSpec::UserProfile,
                EngineSpec::Tag("zh-Hans".into()),
                EngineSpec::Tag("en-US".into())
            ]
        );
        // 显式指定却缺语言包 → 报错；auto 不报错
        assert!(candidate_engines("zh", &["fr-FR".to_string()]).is_err());
        assert_eq!(
            candidate_engines("auto", &["fr-FR".to_string()]).unwrap(),
            vec![EngineSpec::UserProfile]
        );
    }

    #[test]
    fn attempt_plan_is_bounded_and_prefers_primary() {
        let one = vec![EngineSpec::UserProfile];
        assert_eq!(
            attempt_plan(&one),
            vec![(0, PrepMode::Stretch), (0, PrepMode::Binarize)]
        );
        let two = vec![
            EngineSpec::Tag("zh-Hans".into()),
            EngineSpec::Tag("en-US".into()),
        ];
        assert_eq!(
            attempt_plan(&two),
            vec![
                (0, PrepMode::Stretch),
                (0, PrepMode::Binarize),
                (1, PrepMode::Stretch)
            ]
        );
        let three = vec![
            EngineSpec::Tag("a".into()),
            EngineSpec::Tag("b".into()),
            EngineSpec::Tag("c".into()),
        ];
        assert_eq!(attempt_plan(&three).len(), MAX_ATTEMPTS);
        assert!(attempt_plan(&three).iter().all(|(i, _)| *i < 3));
    }
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cd src-tauri && cargo test candidate_engines_primary_then_alternates attempt_plan_is_bounded`

Expected: 编译失败（这三个符号不存在）

- [ ] **Step 3: 实现候选与编排**

在 `preprocess` 之后追加：

```rust
/// 候选 OCR 引擎
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum EngineSpec {
    /// 跟随用户语言（ocr_lang = "auto"）
    UserProfile,
    /// 指定语言标签，如 "zh-Hans"、"en-US"
    Tag(String),
}

impl EngineSpec {
    pub(crate) fn name(&self) -> String {
        match self {
            EngineSpec::UserProfile => "跟随系统".to_string(),
            EngineSpec::Tag(t) => t.clone(),
        }
    }
}

/// 候选引擎：显式 zh/en 时该语言在前、另一种语言兜底；
/// auto 时跟随系统语言在前，其后是可用的中英文备用引擎（中英混排、异体字场景用得上）。
pub(crate) fn candidate_engines(
    lang: &str,
    available: &[String],
) -> Result<Vec<EngineSpec>, String> {
    let mut out: Vec<EngineSpec> = Vec::new();
    match lang {
        "zh" | "en" => {
            let tag = pick_language(available, lang).ok_or_else(|| {
                format!("系统未安装 {lang} 对应的 OCR 语言包，请在 Windows 设置 → 时间和语言 → 语言和区域 中添加")
            })?;
            out.push(EngineSpec::Tag(tag));
        }
        _ => out.push(EngineSpec::UserProfile),
    }
    for alt in ["zh", "en"] {
        if let Some(tag) = pick_language(available, alt) {
            if !out.contains(&EngineSpec::Tag(tag.clone())) {
                out.push(EngineSpec::Tag(tag));
            }
        }
    }
    Ok(out)
}

/// 打分达到该值就不再继续尝试其它组合
pub(crate) const GOOD_SCORE: f64 = 0.8;
/// 单次截图的识别次数上限（每次约 100~400ms）
pub(crate) const MAX_ATTEMPTS: usize = 4;

/// 识别尝试序列（引擎下标, 预处理方式）：主引擎先试两种预处理，
/// 仍不理想才换备用引擎（只试拉伸变体），总次数不超过 MAX_ATTEMPTS
pub(crate) fn attempt_plan(engines: &[EngineSpec]) -> Vec<(usize, PrepMode)> {
    let mut plan = vec![(0usize, PrepMode::Stretch), (0, PrepMode::Binarize)];
    for i in 1..engines.len() {
        if plan.len() >= MAX_ATTEMPTS {
            break;
        }
        plan.push((i, PrepMode::Stretch));
    }
    plan
}

/// 对外门面：签名与错误语义保持不变，内部改成多引擎多预处理择优
#[cfg(windows)]
pub fn recognize(png: &[u8], lang: &str) -> Result<Vec<OcrLine>, String> {
    Ok(recognize_report(png, lang)?.lines)
}

/// 带明细的识别（fixture 人工对照与日志用）
#[cfg(windows)]
pub(crate) struct OcrReport {
    pub lines: Vec<OcrLine>,
    pub score: f64,
    pub engine: String,
    pub mode: &'static str,
    pub attempts: usize,
}

#[cfg(windows)]
pub(crate) fn recognize_report(png: &[u8], lang: &str) -> Result<OcrReport, String> {
    let available = available_languages()?;
    let engines = candidate_engines(lang, &available)?;
    let prepped = preprocess(png)?;
    let bitmaps = decode_variants(&prepped)?;
    let plan = attempt_plan(&engines);

    let mut best: Option<OcrReport> = None;
    let mut attempts = 0usize;
    let mut last_err: Option<String> = None;
    for (idx, mode) in plan {
        let Some((_, bitmap)) = bitmaps.iter().find(|(m, _)| *m == mode) else {
            continue;
        };
        let engine = match create_engine(&engines[idx]) {
            Ok(e) => e,
            Err(e) => {
                // 主引擎建不起来（缺语言包）要如实报错，不能被备用引擎掩盖
                if idx == 0 {
                    return Err(e);
                }
                last_err = Some(e);
                continue;
            }
        };
        attempts += 1;
        match run_engine(&engine, bitmap) {
            Ok(mut lines) => {
                let text = lines
                    .iter()
                    .map(|l| l.text.as_str())
                    .collect::<Vec<_>>()
                    .join("\n");
                let score = score_text(&text);
                if best.as_ref().map_or(true, |b| score > b.score) {
                    // 坐标还原回原裁剪图坐标系（OCR 用的是放大后的图）
                    scale_lines(&mut lines, prepped.scale);
                    best = Some(OcrReport {
                        lines,
                        score,
                        engine: engines[idx].name(),
                        mode: mode.name(),
                        attempts,
                    });
                }
                if score >= GOOD_SCORE {
                    break;
                }
            }
            Err(e) => {
                if idx == 0 && attempts == 1 {
                    return Err(e);
                }
                last_err = Some(e);
            }
        }
    }

    match best {
        Some(mut r) => {
            r.attempts = attempts;
            eprintln!(
                "[ocr] lang={lang} engine={} mode={} score={:.3} attempts={}",
                r.engine, r.mode, r.score, r.attempts
            );
            Ok(r)
        }
        None => Err(last_err.unwrap_or_else(|| "OCR 未返回结果".into())),
    }
}
```

- [ ] **Step 4: 改造 Windows 执行区**

把现有 `recognize` 里「PNG 字节写入内存流 → 解码 → 识别」的那段整体重构为下面三个函数（`mod tests` 之外的 `#[cfg(windows)]` 区域），并把旧的 `recognize` 删除：

```rust
/// 枚举系统已安装的 OCR 语言标签
#[cfg(windows)]
fn available_languages() -> Result<Vec<String>, String> {
    use windows::Media::Ocr::OcrEngine;
    let view = OcrEngine::AvailableRecognizerLanguages()
        .map_err(|e| format!("枚举 OCR 语言失败: {e}"))?;
    let n = view.Size().map_err(|e| e.to_string())?;
    let mut tags: Vec<String> = Vec::with_capacity(n as usize);
    for i in 0..n {
        let l = view.GetAt(i).map_err(|e| e.to_string())?;
        tags.push(l.LanguageTag().map_err(|e| e.to_string())?.to_string());
    }
    Ok(tags)
}

/// 按候选创建引擎
#[cfg(windows)]
fn create_engine(spec: &EngineSpec) -> Result<windows::Media::Ocr::OcrEngine, String> {
    use windows::core::HSTRING;
    use windows::Globalization::Language;
    use windows::Media::Ocr::OcrEngine;
    match spec {
        EngineSpec::UserProfile => OcrEngine::TryCreateFromUserProfileLanguages().map_err(|_| {
            "系统未安装可用的 OCR 语言包，请在 Windows 设置 → 时间和语言 → 语言和区域 中添加"
                .to_string()
        }),
        EngineSpec::Tag(tag) => {
            let language = Language::CreateLanguage(&HSTRING::from(tag.as_str()))
                .map_err(|e| format!("创建语言对象失败: {e}"))?;
            OcrEngine::TryCreateFromLanguage(&language)
                .map_err(|e| format!("创建 OCR 引擎失败: {e}"))
        }
    }
}

/// PNG 字节 → 可直接送 OCR 的 Bgra8 位图。
/// 显式转 Bgra8：预处理后会产生灰度 PNG，而引擎只接受 Bgra8。
#[cfg(windows)]
fn decode_bitmap(png: &[u8]) -> Result<windows::Graphics::Imaging::SoftwareBitmap, String> {
    use windows::Graphics::Imaging::{
        BitmapAlphaMode, BitmapDecoder, BitmapPixelFormat, SoftwareBitmap,
    };
    use windows::Storage::Streams::{DataWriter, InMemoryRandomAccessStream};

    let stream = InMemoryRandomAccessStream::new().map_err(|e| e.to_string())?;
    let writer = DataWriter::CreateDataWriter(&stream).map_err(|e| e.to_string())?;
    writer.WriteBytes(png).map_err(|e| e.to_string())?;
    writer.StoreAsync().map_err(|e| e.to_string())?.get().map_err(|e| e.to_string())?;
    writer.FlushAsync().map_err(|e| e.to_string())?.get().map_err(|e| e.to_string())?;
    stream.Seek(0).map_err(|e| e.to_string())?;

    let decoder = BitmapDecoder::CreateAsync(&stream)
        .map_err(|e| e.to_string())?
        .get()
        .map_err(|e| e.to_string())?;
    let decoded = decoder
        .GetSoftwareBitmapAsync()
        .map_err(|e| e.to_string())?
        .get()
        .map_err(|e| e.to_string())?;
    SoftwareBitmap::ConvertWithAlpha(&decoded, BitmapPixelFormat::Bgra8, BitmapAlphaMode::Premultiplied)
        .map_err(|e| format!("位图格式转换失败: {e}"))
}

/// 把全部变体解码成位图（每个变体只解码一次）
#[cfg(windows)]
fn decode_variants(
    prepped: &Prepped,
) -> Result<Vec<(PrepMode, windows::Graphics::Imaging::SoftwareBitmap)>, String> {
    prepped
        .variants
        .iter()
        .map(|v| Ok((v.mode, decode_bitmap(&v.png)?)))
        .collect()
}

/// 跑一次识别，返回原始（未还原坐标的）文本行
#[cfg(windows)]
fn run_engine(
    engine: &windows::Media::Ocr::OcrEngine,
    bitmap: &windows::Graphics::Imaging::SoftwareBitmap,
) -> Result<Vec<OcrLine>, String> {
    let result = engine
        .RecognizeAsync(bitmap)
        .map_err(|e| e.to_string())?
        .get()
        .map_err(|e| e.to_string())?;

    // 每行包围盒 = 行内所有词矩形并集
    let view = result.Lines().map_err(|e| e.to_string())?;
    let count = view.Size().map_err(|e| e.to_string())?;
    let mut lines = Vec::with_capacity(count as usize);
    for i in 0..count {
        let line = view.GetAt(i).map_err(|e| e.to_string())?;
        let text = line.Text().map_err(|e| e.to_string())?.to_string();
        let words = line.Words().map_err(|e| e.to_string())?;
        let wn = words.Size().map_err(|e| e.to_string())?;
        let mut x0 = f32::MAX;
        let mut y0 = f32::MAX;
        let mut x1 = f32::MIN;
        let mut y1 = f32::MIN;
        for j in 0..wn {
            let r = words
                .GetAt(j)
                .map_err(|e| e.to_string())?
                .BoundingRect()
                .map_err(|e| e.to_string())?;
            x0 = x0.min(r.X);
            y0 = y0.min(r.Y);
            x1 = x1.max(r.X + r.Width);
            y1 = y1.max(r.Y + r.Height);
        }
        if x1 < x0 {
            continue;
        }
        lines.push(OcrLine {
            text,
            x: x0 as i32,
            y: y0 as i32,
            w: (x1 - x0).ceil() as i32,
            h: (y1 - y0).ceil() as i32,
        });
    }
    Ok(lines)
}
```

- [ ] **Step 5: 跑测试确认通过**

Run: `cd src-tauri && cargo test`

Expected: 全部 PASS

- [ ] **Step 6: 实机验证（截图链路仍可用）**

Run: `npm run tauri dev`，`Ctrl+Alt+S` 框选一段普通中英文：

1. 卡片正常出结果（说明新管线的引擎创建/位图转换/识别都通）
2. 终端日志出现 `[ocr] lang=auto engine=… mode=… score=… attempts=…`
3. 常见浅底深字截图的 `attempts` 应为 1（早退生效，速度不退化）
4. 深色主题截图（如 VS Code 暗色）识别正常（反色生效）

- [ ] **Step 7: 提交**

```bash
git add src-tauri/src/ocr.rs
git commit -m "feat: OCR 多引擎多预处理择优（打分选优 + 次数预算 + Bgra8 统一转换）"
```

---

### Task 3: 样本对照工具与人工验收协议

**Files:**
- Create: `src-tauri/tests/fixtures/README.md`
- Modify: `src-tauri/src/ocr.rs`（新增 `#[ignore]` 的 fixture 报告测试）
- Modify: `README.md`（局限说明更新）

**Interfaces:**
- Consumes: Task 2 的 `recognize_report`
- Produces: `#[ignore]` 测试 `ocr_fixture_report`（人工运行，打印每个样本的引擎/模式/得分/文本）

- [ ] **Step 1: 写 fixture 目录说明**

创建 `src-tauri/tests/fixtures/README.md`：

```markdown
# OCR 样本（人工对照用，不入库样本图）

把待测 PNG 放在本目录（`src-tauri/tests/fixtures/*.png`），然后：

```bash
# 改造后（含放大/反色/拉伸/二值化/多引擎择优）
cd src-tauri && cargo test ocr_fixture_report -- --ignored --nocapture

# 对照：跳过全部预处理（等价改造前的原始图直送）
WORDPEEK_OCR_RAW=1 cargo test ocr_fixture_report -- --ignored --nocapture
```

输出每个样本的 `engine / mode / score / attempts` 与识别文本，逐样本比对。

## 建议的 4 类样本（每类 1~2 张，用 Win+Shift+S 截图另存）

| 类别 | 取材建议 | 关注点 |
|---|---|---|
| 复杂背景 | 网页图片上的文字、彩色海报 | 背景纹理/渐变是否被二值化压掉 |
| 深底浅字 | VS Code 暗色主题、终端 | 反色是否生效 |
| 小字号 | PDF 缩略图、小号 UI 文案 | 放大倍数是否足够 |
| 中英混排 / 特殊字体 | 中英夹排文档、衬线/斜体字 | 备用语言引擎是否救回 |
```

- [ ] **Step 2: 写 fixture 报告测试**

`ocr.rs` 的 `mod tests` 追加：

```rust
    #[cfg(windows)]
    #[test]
    #[ignore = "依赖系统 OCR 语言包与本地样本图；手动运行：cargo test ocr_fixture_report -- --ignored --nocapture"]
    fn ocr_fixture_report() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
        let Ok(entries) = std::fs::read_dir(&dir) else {
            println!("样本目录不存在: {}（说明见该目录 README）", dir.display());
            return;
        };
        let raw = std::env::var_os("WORDPEEK_OCR_RAW").is_some();
        println!("样本目录: {}（预处理: {}）", dir.display(), if raw { "关闭" } else { "开启" });
        let mut n = 0;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("png") {
                continue;
            }
            let name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
            let bytes = std::fs::read(&path).expect("读取样本失败");
            match recognize_report(&bytes, "auto") {
                Ok(r) => {
                    println!(
                        "=== {name} | engine={} mode={} score={:.3} attempts={} ===",
                        r.engine, r.mode, r.score, r.attempts
                    );
                    for l in &r.lines {
                        println!("  {}", l.text);
                    }
                }
                Err(e) => println!("=== {name} | 失败: {e} ==="),
            }
            n += 1;
        }
        println!("共 {n} 个样本");
    }
```

- [ ] **Step 3: 跑测试**

Run: `cd src-tauri && cargo test`（`#[ignore]` 的测试默认不跑）

Expected: 全部 PASS，无新增失败

- [ ] **Step 4: 更新 README 局限说明**

`README.md` 的「**局限**」段替换为：

```markdown
**局限**：仅 Windows；混合 DPI 多屏场景尽力而为。OCR 由 Windows 系统引擎完成（离线、免 Key），已做小图放大、深底反色、分位拉伸与自适应二值化、多语言引擎择优；但艺术字体、手写体、严重模糊或强烈反光的图片仍可能识别不准。
```

- [ ] **Step 5: 提交**

```bash
git add src-tauri/tests/fixtures/README.md src-tauri/src/ocr.rs README.md
git commit -m "test: 新增 OCR 样本对照工具与人工验收说明"
```

---

### Task 4: 执行人工对照测量（本计划的验收关口）

**Files:** 无代码改动（只产出测量记录，写在下方表格里并贴到 PR 描述）

- [ ] **Step 1: 收集样本**

按 `src-tauri/tests/fixtures/README.md` 的 4 类各截 1~2 张 PNG，放进 `src-tauri/tests/fixtures/`（样本图不入库）。

- [ ] **Step 2: 跑两遍对照**

```bash
cd src-tauri
cargo test ocr_fixture_report -- --ignored --nocapture                     # 改造后
WORDPEEK_OCR_RAW=1 cargo test ocr_fixture_report -- --ignored --nocapture  # 等价改造前
```

- [ ] **Step 3: 记录并判定**

对每个样本记录：`score（改造前 → 改造后）`、`人工判定的错误字符数（改造前 → 改造后）`、命中的引擎/模式。

**达标判据（全部满足才算通过）：**

1. 复杂背景、深底浅字、小字号三类样本，人工判定错误字符数**均不增加**，且至少两类明显下降
2. 所有样本 `score` 不下降超过 0.02，平均 `score` 上升
3. 常见浅底深字样本 `attempts == 1`（早退生效，日常截图速度不退化）
4. `Ctrl+Alt+S` 实机截图全程正常（无「创建 OCR 引擎失败」「位图格式转换失败」日志）

**未达标时：** 只做参数与组合的调整，不换引擎（用户已明确不接受安装包变大）：

- 调 `upscale_factor` 的目标短边（300 → 400）与倍数上限
- 调 `adaptive_binarize` 的偏置 `c`（8 → 6/12）、窗口半径（短边/32 → /24）
- 调二值化前的高斯模糊 σ（0.8 → 1.2），或去掉模糊只留拉伸变体
- 调 `GOOD_SCORE` 早退阈值，给备用引擎更多机会（代价是耗时）
- 每次只动一个参数，用同一批 fixture 重跑对照，把「参数 → 得分」记在本节下方

若调参后仍达不到判据，结论就是**当前系统 OCR 引擎的能力边界**：在 README 里如实标注（已含「艺术字体、手写体、严重模糊或强烈反光的图片仍可能识别不准」），并把样本与测量记录留在 PR 描述里，不引入随包模型。

## 验收清单（对应 issue #2 第 3 条）

- [ ] 复杂背景、深底浅字、小字号、中英混排四类样本的对照测量完成，达标判据通过
- [ ] 常见场景 `attempts == 1`，截图翻译端到端耗时无可感退化
- [ ] `recognize` 对外签名与错误语义不变，截图链路实机可用
- [ ] 缺语言包时仍给出「请在 Windows 设置 → 时间和语言 → 语言和区域 中添加」的可读提示
- [ ] `cargo test` 通过（`ocr_fixture_report` 为 `#[ignore]`，不影响默认测试）
