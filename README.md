# Wordpeek — 极简划词翻译

在任意窗口选中单词，按下快捷键，鼠标旁弹出翻译卡片，点窗外自动隐藏。

**极简**是它的全部设计哲学：不装运行时、不要账号、不收集任何数据、核心功能零 Key 开箱即用；配置只有一个 json 文件；源码一眼看完，没有一层多余抽象。

![](https://img.shields.io/badge/platform-Windows%20%7C%20macOS-blue) ![](https://img.shields.io/badge/license-MIT-green) [![](https://img.shields.io/github/stars/mrzym99/Wordpeek?style=flat&label=star)](https://github.com/mrzym99/Wordpeek/stargazers)

> 如果 Wordpeek 帮你省了点时间，欢迎在右上角点个 ⭐ **Star** —— 那是这个项目唯一的「收入」，也能让更多人找到它。

## 功能

- **划词翻译**：选中文字后光标旁冒出小翻译按钮，点一下就地出卡片（默认开启，可在设置关闭）——图标经系统辅助功能接口直接读选区，**全程不碰剪贴板**；读不到选区的应用按 `Alt+Z` 快捷键抓取，剪贴板会完整备份还原，不会顶掉你复制的内容
- **输入翻译**：`Alt+X` 唤出窗口，打字/粘贴即译，固定译为英文
- **截图翻译**：`Alt+S` 框选屏幕区域，系统 OCR 本地离线识别（免 Key、数据不出本机）后自动翻译
- **词典卡片**：音标（美/英）、词性、多释义、词形变化、同义词可点击跳查
- **翻译源**：内置有道词典 + 必应翻译，全免 Key 开箱即用；可再填自己的百度/腾讯云密钥作额外源，支持多源对比逐行复制。配置存于 `%APPDATA%\com.wordpeek.desktop\config.json`（macOS 为 `~/Library/Application Support/com.wordpeek.desktop/config.json`），**密钥为明文，请勿外传此文件**
- **外观**：跟随系统 / 亮 / 暗三档，切换即时生效
- **自动更新**：设置 → 关于里检查，下载安装后自动重启

## 下载安装

**Windows**：前往 [Releases](../../releases) 下载 `Wordpeek_x.x.x_x64-setup.exe` 安装，或下载免安装便携版 `Wordpeek_x.x.x_x64-portable.exe` 直接运行。

**macOS**（Apple Silicon，macOS 11+）：前往 [Releases](../../releases) 下载 `Wordpeek_x.x.x_aarch64.dmg`，打开后把 Wordpeek 拖进「应用程序」。目前只出 arm64 包，Intel Mac 请[从源码构建](#从源码构建)。

### macOS 权限（首次运行必读）

macOS 不允许应用静默读屏/模拟按键，首次使用需在「系统设置 → 隐私与安全性」授权两项：

| 权限 | 用途 |
|---|---|
| 辅助功能 | 模拟 Cmd+C 读取选中文本 |
| 屏幕录制 | 截图翻译抓取屏幕像素 |

**授权后必须重启应用才生效**；系统弹的「退出并重新打开」对未签名的包常常只退出不重开，用设置 → 系统权限里的「重启应用」按钮，或者自己再打开一次。不想翻设置，终端里这两条最省事：

```sh
# 去隔离（跳过 Gatekeeper）+ 启动 + 直接跳到「辅助功能」面板
xattr -dr com.apple.quarantine /Applications/Wordpeek.app && open -a Wordpeek \
  && open "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility"

# 授权状态卡住时重置记录再重新勾（勾选本身必须手动，命令行改不了 TCC）
tccutil reset Accessibility com.wordpeek.desktop
tccutil reset ScreenCapture com.wordpeek.desktop
```

### ⚠️ 安装时的安全提示

本项目没有代码签名证书（Windows 的 exe 未做数字签名，macOS 只做了 ad-hoc 签名、没有开发者 ID 也未公证），系统会拦截——**这是所有未签名软件的正常现象，不代表文件有问题**。

**macOS**：首次打开会被 Gatekeeper 拦下，提示「Apple 无法验证 “Wordpeek” 是否包含恶意软件」，解除拦截**关键是第 2 步不能跳**：

1. 把 Wordpeek 拖进「应用程序」，双击一次让它被拦下——弹窗里点「取消」或「完成」，**不要点「移到废纸篓」**
2. 打开「系统设置 → 隐私与安全性」，滚到「安全性」区域，这时才会出现「已阻止使用 “Wordpeek”」和「仍要打开」（没被拦过就不会出现这一行，很多人卡在这里）
3. 点「仍要打开」，用 Touch ID 或密码确认
4. 回到「应用程序」再双击一次，这次弹「确定要打开吗？」，点「打开」

> macOS 15 起「右键 → 打开」已不能绕过拦截，必须走上面的步骤。
> 如果提示的是「已损坏，应移到废纸篓」，那是包本身签名不完整，请重新下载最新包。

**Windows**：SmartScreen 点「更多信息 → 仍要运行」；杀毒软件提示选「信任」或加白名单即可。

**为什么可以放心**：全部源码开放，可审阅或自行构建；应用**不收集、不上传、不存储任何个人信息**——只做两件事：把你选中的文本发给翻译接口，读写你自己的配置文件。翻译请求也仅在你按快捷键时发出。

## 使用说明

| 功能 | 默认快捷键 |
|---|---|
| 划词翻译 | `Alt+Z` |
| 划词图标 | 无（拖选或双击选中文字后自动出现，设置里可关） |
| 截图翻译 | `Alt+S` |
| 输入翻译 | `Alt+X` |

所有快捷键可在托盘菜单 →「设置」→「快捷键」里录制新的组合键。macOS 上 `Alt` 即 `⌥ Option`。

**关于划词图标与剪贴板**：图标只在「拖选」或「双击选词」后出现，单击、点光标不算，免得平时打字也弹。图标路径通过系统辅助功能接口直接读选区（浏览器、原生编辑器、备忘录、PDF 阅读器都可以），**不碰剪贴板**；读不到选区的应用（终端、部分 Electron/自绘 UI）不弹图标，此时用 **Alt+Z**——快捷键经复制键抓取：先备份剪贴板原文，读完立刻还原，慢应用（Word、大网页）迟到的写入也会被纠正回来，你在期间自己复制的内容不会被顶掉。Windows 传统控制台（conhost）例外：那里 Ctrl+C 是「中断」不是复制，应用不会注入复制键，因此其中无法划词（Windows Terminal 不受影响）。

## 从源码构建

前置：Node.js 18+、Rust；Windows 需 MSVC 工具链 + [Visual Studio Build Tools](https://visualstudio.microsoft.com/zh-hans/visual-cpp-build-tools/)，macOS 需 Xcode Command Line Tools（`xcode-select --install`）。

```bash
npm install
npm run tauri build   # Windows → target/release/bundle/nsis/；macOS → bundle/macos + dmg
```

> 不要用裸 `cargo build --release`——不会启用 `custom-protocol`，所有窗口会去连不存在的 devUrl（localhost:1420）而白屏。

## 许可证

[MIT](./LICENSE)

## 改名记录

原名 pot-mini，为避免与 [pot-desktop](https://github.com/pot-app/pot-desktop) 项目产生混淆，已更名为 Wordpeek（本项目为独立实现）。
