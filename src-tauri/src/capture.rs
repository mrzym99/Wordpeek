//! 抓屏纯函数：裁剪、PNG 编码、通道转换（平台抓屏实现在 platform/ 模块）

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
    use image::ImageEncoder;
    // Fast + NoFilter：截图场景优先编码速度而非压缩率，本地 IPC 传输体积影响可忽略
    use image::codecs::png::{CompressionType, FilterType};
    image::codecs::png::PngEncoder::new_with_quality(
        std::io::Cursor::new(&mut out),
        CompressionType::Fast,
        FilterType::NoFilter,
    )
        .write_image(rgba, width as u32, height as u32, image::ExtendedColorType::Rgba8)
        .map_err(|e| format!("PNG 编码失败: {e}"))?;
    Ok(out)
}

/// BGRA（GDI/Vision 输出）→ RGBA，alpha 置 255
pub(crate) fn bgra_to_rgba(buf: &mut [u8]) {
    for px in buf.chunks_exact_mut(4) {
        px.swap(0, 2);
        px[3] = 255;
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
pub fn capture_virtual_screen() -> Result<CapturedScreen, String> {
    Err("截图翻译仅支持 Windows 和 macOS".into())
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

    #[test]
    fn bgra_to_rgba_swaps_channels() {
        let mut buf = vec![1, 2, 3, 255, 4, 5, 6, 0];
        bgra_to_rgba(&mut buf);
        assert_eq!(buf, vec![3, 2, 1, 255, 6, 5, 4, 255]);
    }

    /// 手动冒烟：真实抓屏 → PNG → OCR（需系统装 OCR 语言包）。
    /// 运行：cargo test smoke_real_pipeline -- --ignored --nocapture
    #[cfg(any(windows, target_os = "macos"))]
    #[test]
    #[ignore]
    fn smoke_real_pipeline() {
        let screen = crate::platform::capture_virtual_screen().expect("抓屏应成功");
        println!(
            "虚拟屏: {}x{} @({}, {})，字节长度 {}",
            screen.width, screen.height, screen.origin_x, screen.origin_y, screen.rgba.len()
        );
        assert_eq!(
            screen.rgba.len(),
            (screen.width as usize) * (screen.height as usize) * 4
        );

        // 全屏快照落盘，肉眼检查通道顺序/方向是否正确
        let png = encode_png(&screen.rgba, screen.width, screen.height).expect("PNG 编码应成功");
        let path = std::env::temp_dir().join("wordpeek-capture-smoke.png");
        std::fs::write(&path, &png).expect("写盘应成功");
        println!("全屏快照已写入: {}", path.display());

        // 屏幕中央 800x400 区域跑真 OCR（无文字/无语言包时只告警不失败）
        let region = Rect { x: screen.width / 2 - 400, y: screen.height / 2 - 200, w: 800, h: 400 };
        if let Ok(cropped) = crop(&screen.rgba, screen.width, screen.height, region) {
            let small = encode_png(&cropped, region.w, region.h).expect("小图编码应成功");
            match crate::ocr::recognize(&small, "auto") {
                Ok(lines) => {
                    println!("OCR 识别 {} 行:", lines.len());
                    for l in lines.iter().take(10) {
                        println!("  {:?}", l.text);
                    }
                }
                Err(e) => println!("OCR 跳过（系统可能未装语言包）: {e}"),
            }
        }
    }
}
