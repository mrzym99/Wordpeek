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
}

/// PNG 字节 → OCR 文本行。lang: "auto" | "zh" | "en"
#[cfg(windows)]
pub fn recognize(png: &[u8], lang: &str) -> Result<Vec<OcrLine>, String> {
    use windows::core::HSTRING;
    use windows::Globalization::Language;
    use windows::Graphics::Imaging::BitmapDecoder;
    use windows::Media::Ocr::OcrEngine;
    use windows::Storage::Streams::{DataWriter, InMemoryRandomAccessStream};

    // 按语言挑引擎："zh"/"en" 显式挑语言包，"auto" 跟随用户语言
    let engine = unsafe {
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

    unsafe {
        // PNG 字节写入内存流供解码器读取
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
