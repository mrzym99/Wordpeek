#![allow(dead_code)] // Task 5b 接入截图流程后移除
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
    fn upscale_factor_reaches_min_side() {
        assert_eq!(upscale_factor(50, 20), 4); // 150/20=7.5 → 封顶 4
        assert_eq!(upscale_factor(120, 80), 2); // 150/80=1.875 → 2
        assert_eq!(upscale_factor(800, 400), 1); // 已够大
        assert_eq!(upscale_factor(2000, 40), 4); // 宽扁条也放大
    }

    #[cfg(windows)]
    #[test]
    fn upscale_if_small_enlarges_image() {
        use crate::capture::encode_png;
        let rgba = vec![255u8; 50 * 20 * 4];
        let png = encode_png(&rgba, 50, 20).expect("编码应成功");
        let out = upscale_if_small(&png).expect("放大应成功");
        let img = image::load_from_memory(&out).expect("解码应成功");
        assert_eq!((img.width(), img.height()), (200, 80));
        // 已够大的图原样返回
        let big = encode_png(&vec![255u8; 800 * 400 * 4], 800, 400).expect("编码应成功");
        let same = upscale_if_small(&big).expect("应成功");
        assert_eq!(image::load_from_memory(&same).unwrap().width(), 800);
    }
}

/// 小图放大倍数：短边放大到至少 150px，2~4 倍；已够大则返回 1。
/// Windows OCR 对小字号/小图识别率极低，放大后显著改善。
fn upscale_factor(w: u32, h: u32) -> u32 {
    let min = w.min(h);
    if min == 0 {
        return 1;
    }
    (150.0 / min as f32).ceil().clamp(1.0, 4.0) as u32
}

/// 小图预处理：短边不足 150px 的裁剪图放大 2~4 倍再交给 OCR
#[cfg(windows)]
fn upscale_if_small(png: &[u8]) -> Result<Vec<u8>, String> {
    let img = image::load_from_memory(png).map_err(|e| format!("PNG 解码失败: {e}"))?;
    let f = upscale_factor(img.width(), img.height());
    if f == 1 {
        return Ok(png.to_vec());
    }
    let scaled = image::imageops::resize(
        &img,
        img.width() * f,
        img.height() * f,
        image::imageops::FilterType::Lanczos3,
    );
    let mut out = Vec::new();
    image::DynamicImage::ImageRgba8(scaled)
        .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
        .map_err(|e| format!("PNG 编码失败: {e}"))?;
    Ok(out)
}

/// PNG 字节 → OCR 文本行。lang: "auto" | "zh" | "en"
#[cfg(windows)]
pub fn recognize(png: &[u8], lang: &str) -> Result<Vec<OcrLine>, String> {
    use windows::core::HSTRING;
    // 小图先放大再识别，否则 Windows OCR 对小字号基本识别不出
    let png = upscale_if_small(png)?;
    use windows::Globalization::Language;
    use windows::Graphics::Imaging::BitmapDecoder;
    use windows::Media::Ocr::OcrEngine;
    use windows::Storage::Streams::{DataWriter, InMemoryRandomAccessStream};

    // 按语言挑引擎："zh"/"en" 显式挑语言包，"auto" 跟随用户语言
    let engine = {
        match lang {
            "zh" | "en" => {
                let view = OcrEngine::AvailableRecognizerLanguages()
                    .map_err(|e| format!("枚举 OCR 语言失败: {e}"))?;
                let n = view.Size().map_err(|e| e.to_string())?;
                let mut tags: Vec<String> = Vec::with_capacity(n as usize);
                for i in 0..n {
                    let l = view.GetAt(i).map_err(|e| e.to_string())?;
                    tags.push(l.LanguageTag().map_err(|e| e.to_string())?.to_string());
                }
                let tag = pick_language(&tags, lang).ok_or_else(|| {
                    format!("系统未安装 {lang} 对应的 OCR 语言包，请在 Windows 设置 → 时间和语言 → 语言和区域 中添加")
                })?;
                let language = Language::CreateLanguage(&HSTRING::from(tag))
                    .map_err(|e| format!("创建语言对象失败: {e}"))?;
                OcrEngine::TryCreateFromLanguage(&language)
                    .map_err(|e| format!("创建 OCR 引擎失败: {e}"))?
            }
            _ => OcrEngine::TryCreateFromUserProfileLanguages()
                .map_err(|_| "系统未安装可用的 OCR 语言包，请在 Windows 设置中添加语言".to_string())?,
        }
    };

    // PNG 字节写入内存流供解码器读取
    {
        let stream = InMemoryRandomAccessStream::new().map_err(|e| e.to_string())?;
        let writer = DataWriter::CreateDataWriter(&stream).map_err(|e| e.to_string())?;
        writer.WriteBytes(&png).map_err(|e| e.to_string())?;
        writer.StoreAsync().map_err(|e| e.to_string())?.get().map_err(|e| e.to_string())?;
        writer.FlushAsync().map_err(|e| e.to_string())?.get().map_err(|e| e.to_string())?;
        stream.Seek(0).map_err(|e| e.to_string())?;

        let decoder = BitmapDecoder::CreateAsync(&stream)
            .map_err(|e| e.to_string())?
            .get()
            .map_err(|e| e.to_string())?;
        let bitmap = decoder
            .GetSoftwareBitmapAsync()
            .map_err(|e| e.to_string())?
            .get()
            .map_err(|e| e.to_string())?;
        let result = engine
            .RecognizeAsync(&bitmap)
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
}

#[cfg(not(windows))]
pub fn recognize(_png: &[u8], _lang: &str) -> Result<Vec<OcrLine>, String> {
    Err("截图翻译仅支持 Windows".into())
}
