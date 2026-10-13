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
