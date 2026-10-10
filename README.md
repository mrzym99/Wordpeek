# Wordpeek — 最小划词翻译（Tauri + React）

在任意窗口选中单词，按下快捷键，鼠标旁弹出翻译卡片；点击窗外自动隐藏。

![](https://img.shields.io/badge/platform-Windows-blue) ![](https://img.shields.io/badge/license-MIT-green)

## 功能

- **划词翻译**：全局快捷键抓取选中文本，鼠标位置弹出翻译卡片
- **丰富的词典卡片**：音标（美/英）、词性、多释义、词形变化、同义词（可点击跳查）、考试标签
- **多翻译源**：有道词典（免 Key）/ 百度翻译；主源失败自动降级
- **可配置**：独立设置窗口——翻译源、API 密钥、快捷键（支持录制组合键）
- **系统托盘**：常驻后台，托盘菜单打开设置或退出

## 下载安装

前往 [Releases](../../releases) 下载 `Wordpeek_x.x.x_x64-setup.exe` 安装，或下载便携版 `wordpeek.exe` 直接运行。

### ⚠️ 安装时的安全提示

本项目未购买代码签名证书，安装包（exe）没有数字签名。Windows SmartScreen 和部分杀毒软件可能会弹出"Windows 已保护你的电脑"或"未知发布者"等警告——**这是所有未签名软件的正常现象，不代表文件有问题**。

如需继续安装：

- **SmartScreen**：点击"更多信息" → "仍要运行"
- **杀毒软件拦截**：在拦截提示中选择"信任"或"添加白名单"

**为什么可以放心安装**：本项目的全部源码都在本仓库中开放，可以自行审阅或从源码构建；应用**不收集、不上传、不存储任何用户个人信息**——它只做两件事：读取你选中的文本发给翻译接口（有道/百度），以及读写你自己的配置文件。没有遥测、没有统计上报、没有后台联网行为（翻译请求仅在你按快捷键时发出）。

如果你仍然不放心，可以下载便携版后用杀毒软件扫描，或直接[从源码构建](#从源码构建)。

> Windows SmartScreen 的拦截与文件是否真的危险无关，只与"是否有人举报过该文件、下载量是否足够"有关。小众软件几乎必然被拦截，代码签名证书价格昂贵（每年数千元），个人开源项目通常无力承担。

## 从源码构建

前置：Node.js 18+、Rust（MSVC 工具链）+ [Visual Studio Build Tools](https://visualstudio.microsoft.com/zh-hans/visual-cpp-build-tools/)

```bash
npm install
npm run tauri dev     # 开发模式
npm run tauri build   # 打包，产物在 src-tauri/target/release/bundle/nsis/
```

## 使用说明

1. 启动后应用常驻系统托盘（任务栏右下角，可能在 `^` 折叠区里）
2. 在任意窗口选中单词，按 **Ctrl+Alt+T**（默认，可在设置中录制新的组合键）
3. 翻译卡片出现在鼠标旁，点击同义词可在卡片内继续跳查
4. 托盘菜单或卡片右上角 ⚙ 打开设置

翻译源配置：

| 翻译源 | 配置 |
|---|---|
| 有道词典 | 免 Key，开箱即用 |
| 百度翻译 | [百度翻译开放平台](https://fanyi-api.baidu.com) 申请 appid + 密钥 |

配置保存于 `%APPDATA%\com.wordpeek.app\config.json`（密钥为明文，请勿外传此文件）。

## 原理

1. Rust 通过 `global-shortcut` 插件注册全局快捷键
2. 触发时：备份剪贴板 → 写入哨兵值 → 模拟 Ctrl+C → 读取剪贴板 → 还原剪贴板
3. `GetCursorPos` 取鼠标坐标，emit 事件给前端
4. React 收到事件后：移动弹窗到鼠标旁 → show → Rust 端请求翻译接口 → 渲染词典卡片

（翻译请求全部由 Rust 端发出，避免 webview CORS 限制，API 密钥不进前端。）

## 目录

```
src/App.tsx            # 弹窗 UI + 设置窗口 + 事件监听
src-tauri/src/main.rs  # 全局快捷键、模拟 Ctrl+C、剪贴板、光标坐标、托盘
src-tauri/src/translate.rs  # 翻译源（有道/百度）、配置读写
src-tauri/tauri.conf.json   # 无边框/置顶/透明弹窗 + 独立设置窗口
```

## 许可证

[MIT](./LICENSE)

## 改名记录

原名 pot-mini，为避免与 [pot-desktop](https://github.com/pot-app/pot-desktop) 项目产生混淆，已更名为 Wordpeek（本项目为独立实现）。首次启动会自动把旧目录（`%APPDATA%\com.potmini.app`）的配置迁移到新目录。

## 截图翻译

按 `Ctrl+Alt+S`(可在设置中修改)框选屏幕任意区域:

1. 全屏遮罩出现并冻结当前画面，拖拽框选，ESC 或点击窗外取消
2. Windows 系统 OCR 本地识别文字(免 Key、离线、数据不出本机)
3. 识别结果自动翻译，弹出双语卡片，支持一键复制原文/译文

**OCR 语言**:设置 → 截图翻译 → 识别语言(跟随系统语言 / 中文优先 / 英文优先)。语言包随 Windows 安装(设置 → 时间和语言 → 语言和区域)。

**注意**:单词查询免 Key(有道词条);**整句翻译需在设置中配置百度翻译密钥**(有道已下线免费整句接口)。

**局限**:仅 Windows;混合 DPI 多屏场景尽力而为。

| 功能 | 默认快捷键 |
|---|---|
| 划词翻译 | `Ctrl+Alt+T` |
| 截图翻译 | `Ctrl+Alt+S` |
