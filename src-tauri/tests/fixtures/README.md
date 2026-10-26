# OCR 样本（人工对照用）

把待测 PNG 放在本目录（`src-tauri/tests/fixtures/*.png`），然后：

```bash
cd src-tauri && cargo test ocr_fixture_report -- --ignored --nocapture
```

- macOS：输出每行文本与 `[宽x高+左+上]` 物理像素坐标（图片尺寸为 PNG 真实像素）
- Windows：输出每个样本的 `engine / mode / score / attempts` 与识别文本，逐样本比对；加 `WORDPEEK_OCR_RAW=1` 可跳过全部预处理做对照

> 只提交一张 `ocr_hello.png` 作为冒烟样本（macOS Vision 首次调用会加载模型，可能耗十几秒，第二次起 ~0.2s）。其余真实截图不入库。

## 旧版 Windows 样本建议（未入库）

| 类别 | 取材建议 | 关注点 |
|---|---|---|
| 复杂背景 | 网页图片上的文字、彩色海报 | 背景纹理/渐变是否被二值化压掉 |
| 深底浅字 | VS Code 暗色主题、终端 | 反色是否生效 |
| 小字号 | PDF 缩略图、小号 UI 文案 | 放大倍数是否足够 |
| 中英混排 / 特殊字体 | 中英夹排文档、衬线/斜体字 | 备用语言引擎是否救回 |
