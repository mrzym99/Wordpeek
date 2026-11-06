# Wordpeek — 极简划词翻译

在任意窗口选中单词，弹出翻译卡片自动翻译。

**极简** 开箱即用，配置极少

![](https://img.shields.io/badge/platform-Windows%20%7C%20macOS-blue) ![](https://img.shields.io/badge/license-MIT-green) [![](https://img.shields.io/github/stars/mrzym99/Wordpeek?style=flat&label=star)](https://github.com/mrzym99/Wordpeek/stargazers)

> 如果 Wordpeek 对你确实有帮助，欢迎在右上角点个 ⭐ **Star** —— 希望能让更多人找到它。

## 功能

- **划词翻译**：选中文字后按 `Alt+Z` 快捷键/点击图标翻译（mac 上将 alt 替换为 ⌥ Option即可）
- **截图翻译**：`Alt+S` 框选屏幕区域，系统 OCR 本地离线识别
- **输入翻译**：`Alt+X` 唤出窗口，打字/粘贴即译
- **翻译源**：内置有道词典 + 必应翻译，全免 Key 开箱即用；可在设置补充额外翻译源
- **外观**：跟随系统/浅色/深色
- **自动更新**：设置 → 关于里检查。Windows 下载安装后自动重启；macOS 可检查新版本，前往 Releases 下载安装包

## 下载安装

**Windows**：前往 [Releases](../../releases) 下载 `Wordpeek_x.x.x_x64-setup.exe` 安装。

**macOS**（Apple Silicon，macOS 11+）：前往 [Releases](../../releases) 下载 `Wordpeek_x.x.x_aarch64.dmg` 安装。（目前只出 arm64 包，支持 m 系列芯片）

### macOS 权限（首次运行必读）

macOS 首次使用需在「系统设置 → 隐私与安全性」授权两项：

| 权限 | 用途 |
|---|---|
| 辅助功能 | 读取选中文本 |
| 屏幕录制 | 截图翻译 |

**授权后必须重启应用才生效**

不想手动设置权限，通过终端打开设置最方便：
```sh
# 去隔离（跳过 Gatekeeper）+ 启动 + 直接跳到权限面板（先辅助功能、后屏幕录制）
xattr -dr com.apple.quarantine /Applications/Wordpeek.app && open -a Wordpeek \
  && open "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility" \
  && open "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture"

# 授权状态卡住时重置记录再重新勾（勾选本身必须手动，命令行改不了 TCC）
tccutil reset Accessibility com.wordpeek.desktop
tccutil reset ScreenCapture com.wordpeek.desktop
```

### ⚠️ 安装时的安全提示

本项目没有代码签名证书，系统会拦截——**这是所有未签名软件的正常现象，不代表文件有问题**。

**macOS**：首次打开会被 Gatekeeper 拦下，提示「Apple 无法验证 “Wordpeek” 是否包含恶意软件」，解除拦截**关键是第 2 步不能跳**：

1. 把 Wordpeek 拖进「应用程序」，双击一次让它被拦下——弹窗里点「取消」或「完成」，**不要点「移到废纸篓」**
2. 打开「系统设置 → 隐私与安全性」，滚到「安全性」区域，这时才会出现「已阻止使用 “Wordpeek”」和「仍要打开」（没被拦过就不会出现这一行，很多人卡在这里）
3. 点「仍要打开」

**Windows**：SmartScreen 点「更多信息 → 仍要运行」；杀毒软件提示选「信任」或加白名单即可。

**为什么可以放心**：全部源码开放，可审阅或自行构建；应用**不收集、不上传、不存储任何个人信息**

## 从源码构建

前置：Node.js 18+、Rust；Windows 需 MSVC 工具链 + [Visual Studio Build Tools](https://visualstudio.microsoft.com/zh-hans/visual-cpp-build-tools/)，macOS 需 Xcode Command Line Tools（`xcode-select --install`）。

```bash
npm install
npm run tauri build
```

## 许可证

[MIT](./LICENSE)
