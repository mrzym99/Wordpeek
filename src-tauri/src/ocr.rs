#![allow(dead_code)] // OcrLine 坐标已按契约还原，但尚未被前端消费（留给原位覆盖展示）
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
        for mode in [PrepMode::Stretch, PrepMode::Binarize] {
            let bytes = p.png_for(mode).expect("变体应生成成功").expect("应有该变体");
            let img = image::load_from_memory(&bytes).expect("变体应是合法 PNG");
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
        let p = preprocess(&encode_gray(img).expect("编码应成功")).expect("预处理应成功");
        assert_eq!(p.scale, 1);

        // 拉伸变体：反色生效 → 笔画变深、背景变亮
        let stretch_png = p.png_for(PrepMode::Stretch).expect("生成成功").expect("有变体");
        let stretch = image::load_from_memory(&stretch_png)
            .unwrap()
            .to_luma8();
        assert!(stretch.get_pixel(46, 200)[0] < 64, "反色后笔画应偏黑");
        assert!(stretch.get_pixel(70, 200)[0] > 192, "反色后背景应偏白");

        // 二值化变体：笔画为黑、笔画间隔为白
        let bin_png = p.png_for(PrepMode::Binarize).expect("生成成功").expect("有变体");
        let bin = image::load_from_memory(&bin_png)
            .unwrap()
            .to_luma8();
        assert_eq!(bin.get_pixel(46, 200)[0], 0, "笔画应为黑");
        assert_eq!(bin.get_pixel(70, 200)[0], 255, "笔画间隔应为白");
    }

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

    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "依赖 macOS Vision 与本地样本图；手动运行：cargo test ocr_fixture_report -- --ignored --nocapture"]
    fn ocr_fixture_report() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
        let Ok(entries) = std::fs::read_dir(&dir) else {
            println!("样本目录不存在: {}（说明见该目录 README）", dir.display());
            return;
        };
        let mut n = 0;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("png") {
                continue;
            }
            let name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
            let bytes = std::fs::read(&path).expect("读取样本失败");
            match recognize(&bytes, "auto") {
                Ok(lines) => {
                    println!("=== {name} | {} 行 ===", lines.len());
                    for l in &lines {
                        println!("  [{}x{}+{}+{}] {}", l.w, l.h, l.x, l.y, l.text);
                    }
                }
                Err(e) => println!("=== {name} | 失败: {e} ==="),
            }
            n += 1;
        }
        println!("共 {n} 个样本");
    }

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
}

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

/// 预处理结果：底图 + 按需生成的变体缓存 + 相对原裁剪图的放大倍数（用于把 OCR 坐标还原回去）。
/// 变体懒生成：Stretch 首试通常达标即退，Binarize 的模糊+积分图+编码在常规路径全部免掉（峰值省几十 MB）。
pub(crate) struct Prepped {
    base: Option<image::GrayImage>, // 处理完的底图（放大/反色后）；None = RAW 模式
    variants: std::cell::RefCell<Vec<Prep>>,
    pub scale: u32,
}

impl Prepped {
    /// 取某变体的 PNG，首次用时生成并缓存；RAW 模式下只有缓存的原图变体，其余返回 None
    pub(crate) fn png_for(&self, mode: PrepMode) -> Result<Option<Vec<u8>>, String> {
        if let Some(p) = self.variants.borrow().iter().find(|v| v.mode == mode) {
            return Ok(Some(p.png.clone()));
        }
        let Some(base) = self.base.as_ref() else {
            return Ok(None);
        };
        let png = match mode {
            PrepMode::Stretch => encode_gray(stretch_contrast(base))?,
            PrepMode::Binarize => {
                // 二值化前轻度高斯模糊：压掉 JPEG 噪点与背景纹理，同时保留笔画
                let denoised = image::imageops::blur(base, 0.8);
                encode_gray(adaptive_binarize(&denoised, 8))?
            }
        };
        self.variants.borrow_mut().push(Prep { mode, png: png.clone() });
        Ok(Some(png))
    }
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
    // 积分图带一圈 0 边界：int[y][x] = 左上角 y×x 区域的和。
    // u32 够用：放大后最长边封顶 4000（upscale_factor），总和 ≤ 4000×4000×255 < u32::MAX；
    // 只有容斥四项相减那一步可能越界，显式升 u64。比 u64 积分图省一半峰值内存。
    let mut integral = vec![0u32; (w + 1) * (h + 1)];
    for y in 0..h {
        let mut row_sum = 0u32;
        for x in 0..w {
            row_sum += src[y * w + x] as u32;
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
            let area = ((x1 - x0) * (y1 - y0)) as u32;
            let sum = integral[y1 * (w + 1) + x1] as u64 + integral[y0 * (w + 1) + x0] as u64
                - integral[y0 * (w + 1) + x1] as u64
                - integral[y1 * (w + 1) + x0] as u64;
            if (src[y * w + x] as i32) < (sum / area as u64) as i32 - c {
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
/// 收所有权：借用版要 `img.clone()` 整张图才能包成 DynamicImage
fn encode_gray(img: image::GrayImage) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    image::DynamicImage::ImageLuma8(img)
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
            base: None,
            variants: std::cell::RefCell::new(vec![Prep {
                mode: PrepMode::Stretch,
                png: png.to_vec(),
            }]),
            scale: 1,
        });
    }
    // 一步转灰度：RGBA 原图在本行结束就释放，不跟着后续变体一起活着
    let gray = image::load_from_memory(png)
        .map_err(|e| format!("PNG 解码失败: {e}"))?
        .to_luma8();
    let scale = upscale_factor(gray.width(), gray.height());
    let mut base: image::GrayImage = if scale > 1 {
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

    if should_invert(pixel_mean(&base)) {
        image::imageops::invert(&mut base);
    }

    // 变体不在此处生成：由 recognize_report 按 attempt_plan 按需调 png_for
    Ok(Prepped {
        base: Some(base),
        variants: std::cell::RefCell::new(Vec::new()),
        scale,
    })
}

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
    let plan = attempt_plan(&engines);

    let mut best: Option<OcrReport> = None;
    let mut attempts = 0usize;
    let mut last_err: Option<String> = None;
    for (idx, mode) in plan {
        // 变体按需生成/解码：达标即退出时 Binarize 的模糊+积分图+编码全部不做
        let png = match prepped.png_for(mode)? {
            Some(p) => p,
            None => continue,
        };
        let bitmap = match decode_bitmap(&png) {
            Ok(b) => b,
            Err(e) => {
                // 主变体的解码失败要如实报错，不能被备用引擎掩盖
                if idx == 0 {
                    return Err(e);
                }
                last_err = Some(e);
                continue;
            }
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
        match run_engine(&engine, &bitmap) {
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
    let bitmap = {
        // 解码出的 RGBA 位图用完即弃，不陪着 Bgra8 转换结果活到函数尾（大图瞬时省一半）
        let decoded = decoder
            .GetSoftwareBitmapAsync()
            .map_err(|e| e.to_string())?
            .get()
            .map_err(|e| e.to_string())?;
        let bitmap = SoftwareBitmap::ConvertWithAlpha(
            &decoded,
            BitmapPixelFormat::Bgra8,
            BitmapAlphaMode::Premultiplied,
        )
        .map_err(|e| format!("位图格式转换失败: {e}"))?;
        drop(decoded);
        bitmap
    };
    Ok(bitmap)
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

/// macOS Vision（VNRecognizeTextRequest）：PNG → 文本行（行坐标保留，本期不使用）。
/// Vision 对小字识别好，预处理免掉：原图直接送识别（RAW 直传）#[cfg(target_os = "macos")]
pub fn recognize(png: &[u8], lang: &str) -> Result<Vec<OcrLine>, String> {
    use objc2::rc::Retained;
    use objc2::AnyThread;
    use objc2_foundation::{NSData, NSDictionary, NSArray, NSString};
    use objc2_vision::{
        VNImageRequestHandler, VNRecognizeTextRequest, VNRequest, VNRequestTextRecognitionLevel,
    };

    let (w, h) = {
        let img = image::load_from_memory(png).map_err(|e| format!("PNG 解码失败: {e}"))?;
        (img.width(), img.height())
    };
    if w == 0 || h == 0 {
        return Err("图像尺寸非法".into());
    }

    let data = NSData::with_bytes(png);
    let handler = VNImageRequestHandler::initWithData_options(
        VNImageRequestHandler::alloc(),
        &data,
        &NSDictionary::new(),
    );
    let request = unsafe { VNRecognizeTextRequest::init(VNRecognizeTextRequest::alloc()) };
    request.setRecognitionLevel(VNRequestTextRecognitionLevel::Accurate);
    request.setUsesLanguageCorrection(true);
    // 语言提示：zh 简/繁都提示；en 固定 en-US；auto 不设（引擎自动）
    let langs: Vec<Retained<NSString>> = match lang {
        "zh" => vec![NSString::from_str("zh-Hans"), NSString::from_str("zh-Hant")],
        "en" => vec![NSString::from_str("en-US")],
        _ => vec![],
    };
    if !langs.is_empty() {
        let refs: Vec<&NSString> = langs.iter().map(|s| &**s).collect();
        request.setRecognitionLanguages(&NSArray::from_slice(&refs));
    }

    let reqs = NSArray::from_slice(&[&*request as &VNRequest]);
    let t0 = std::time::Instant::now();
    handler
        .performRequests_error(&reqs)
        .map_err(|e| format!("OCR 识别失败: {e}"))?;
    // 首次调用要加载 Vision 模型，冷启动明显慢于后续
    let vision_ms = t0.elapsed().as_millis();

    let Some(results) = request.results() else {
        return Err("OCR 未返回结果".into());
    };
    let n = results.count();
    let mut lines = Vec::with_capacity(n);
    for i in 0..n {
        let obs = results.objectAtIndex(i);
        let cands = obs.topCandidates(1);
        let Some(t) = cands.firstObject() else { continue };
        // boundingBox 归一化（左下原点）→ 裁剪图物理像素（左上原点）
        let bb = unsafe { obs.boundingBox() };
        lines.push(OcrLine {
            text: t.string().to_string(),
            x: (bb.origin.x * w as f64).round() as i32,
            y: ((1.0 - bb.origin.y - bb.size.height) * h as f64).round() as i32,
            w: (bb.size.width * w as f64).round() as i32,
            h: (bb.size.height * h as f64).round() as i32,
        });
    }
    println!("[ocr] macos vision: {} 行，{}x{}px，用时 {}ms", lines.len(), w, h, vision_ms);
    Ok(lines)
}

#[cfg(not(any(windows, target_os = "macos")))]
pub fn recognize(_png: &[u8], _lang: &str) -> Result<Vec<OcrLine>, String> {
    Err("截图翻译仅支持 Windows 和 macOS".into())
}

/// 预热：进程内第一次调 Vision 要加载识别模型，实测冷启动 30-60s，
/// 正好是「第一次截图翻译卡半分钟」的元凶。启动后后台先跑一次，后续识别降到百毫秒级。
#[cfg(target_os = "macos")]
pub fn warmup() {
    use image::{ImageFormat, Rgba, RgbaImage};
    // 32x32 纯白小图就够触发模型加载
    let img = RgbaImage::from_pixel(32, 32, Rgba([255, 255, 255, 255]));
    let mut buf = std::io::Cursor::new(Vec::new());
    if img.write_to(&mut buf, ImageFormat::Png).is_err() {
        return;
    }
    let t0 = std::time::Instant::now();
    let _ = recognize(&buf.into_inner(), "auto"); // 白图无文字，Err 正常
    println!("[ocr] 预热完成，用时 {}ms", t0.elapsed().as_millis());
}

#[cfg(not(target_os = "macos"))]
pub fn warmup() {}
