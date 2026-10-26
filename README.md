# Wordpeek — 最小划词翻译（Tauri + React）

在任意窗口选中单词，按下快捷键，鼠标旁弹出翻译卡片；点击窗外自动隐藏。

![](https://img.shields.io/badge/platform-Windows%20%7C%20macOS-blue) ![](https://img.shields.io/badge/license-MIT-green)

## 功能

- **划词翻译**：全局快捷键抓取选中文本，鼠标位置弹出翻译卡片
- **输入翻译**：`Ctrl+Alt+I`（或托盘菜单）唤出窗口，打字/粘贴即译，固定译为英文
- **丰富的词典卡片**：音标（美/英）、词性、多释义、词形变化、同义词（可点击跳查）、考试标签
- **翻译策略**：默认源内置（单词→有道词典，句子→必应翻译，免 Key 开箱即用），可在设置里关闭（不能全关，至少保留一个源）；**额外源**：腾讯翻译（腾讯云密钥）、百度翻译（开放平台密钥）——都需先在「源配置」（左侧菜单切换源）填好密钥才能勾选，删密钥自动取消勾选
- **多源对比**：有道/必应视为同一个「默认源」（单词/句子各司其职），对比区不单独出现；额外源（如百度）以浅色背景分区展示在卡片末尾或输入窗口「默认翻译」行之后，逐行可复制
- **截图翻译**：`Ctrl+Alt+S` 框选屏幕区域，系统 OCR 本地离线识别（免 Key）后自动翻译，详见[截图翻译](#截图翻译)
- **可配置**：独立设置窗口——额外翻译源勾选、API 密钥、三个快捷键（支持录制组合键）

## 下载安装

**Windows**：前往 [Releases](../../releases) 下载 `Wordpeek_x.x.x_x64-setup.exe` 安装，或下载免安装便携版 `Wordpeek_x.x.x_x64-portable.exe` 直接运行。

**macOS**（Apple Silicon，macOS 11+）：前往 [Releases](../../releases) 下载 `Wordpeek_x.x.x_aarch64.dmg`，打开后把 Wordpeek 拖进「应用程序」。目前只出 arm64 包，Intel Mac 请[从源码构建](#从源码构建)。

### macOS 权限（首次运行必读）

macOS 不允许应用静默读屏/模拟按键，首次使用需在「系统设置 → 隐私与安全性」授权两项：

| 权限 | 用途 | 位置 |
|---|---|---|
| 辅助功能 | 模拟 Cmd+C 读取选中文本 | 隐私与安全性 → 辅助功能 |
| 屏幕录制 | 截图翻译抓取屏幕像素 | 隐私与安全性 → 屏幕录制 |

设置 → 系统权限 里有当前状态和一键跳转按钮；**授权后需退出并重启应用**（系统只对新进程生效）。

### ⚠️ 安装时的安全提示

本项目没有代码签名证书（Windows 的 exe 未做数字签名，macOS 只做了 ad-hoc 签名、没有开发者 ID 也未公证），系统会拦截——**这是所有未签名软件的正常现象，不代表文件有问题**。

**macOS**：双击 dmg 里的应用可能被拦，提示「无法打开，因为 Apple 无法验证」或「已损坏，应移到废纸篓」。

- 推荐：先把应用拖进「应用程序」，再**右键 → 打开**，在弹窗里点一次「打开」（之后正常双击即可）
- 或者装好后在终端执行一次：`xattr -dr com.apple.quarantine /Applications/Wordpeek.app`

**Windows**：SmartScreen 或部分杀毒软件可能弹「Windows 已保护你的电脑」「未知发布者」。

- **SmartScreen**：点击「更多信息」→「仍要运行」
- **杀毒软件拦截**：在拦截提示中选择「信任」或「添加白名单」

**为什么可以放心安装**：本项目的全部源码都在本仓库中开放，可以自行审阅或从源码构建；应用**不收集、不上传、不存储任何用户个人信息**——它只做两件事：读取你选中的文本发给翻译接口（有道/百度），以及读写你自己的配置文件。没有遥测、没有统计上报、没有后台联网行为（翻译请求仅在你按快捷键时发出）。

如果你仍然不放心，可以下载便携版后用杀毒软件扫描，或直接[从源码构建](#从源码构建)。

> 这类拦截与文件是否真的危险无关：Windows SmartScreen 只看「是否有人举报过、下载量够不够」，macOS 只看「有没有开发者 ID 签名 + 公证」。小众软件几乎必然被拦，而 Windows 代码签名证书与 Apple 开发者账号都要付费（每年数百到数千元），个人开源项目通常无力承担。

## 从源码构建
前置：Node.js 18+、Rust

- Windows：MSVC 工具链 + [Visual Studio Build Tools](https://visualstudio.microsoft.com/zh-hans/visual-cpp-build-tools/)
- macOS：Xcode Command Line Tools（`xcode-select --install`），无需额外依赖（OCR 走系统 Vision，抓屏走 CoreGraphics）

```bash
npm install
npm run tauri dev     # 开发模式
npm run tauri build   # 打包
#   Windows → src-tauri/target/release/bundle/nsis/
#   macOS   → src-tauri/target/release/bundle/macos/Wordpeek.app（+ .dmg）
```

## 使用说明

1. 启动后应用常驻系统托盘（任务栏右下角，可能在 `^` 折叠区里）
2. 在任意窗口选中单词，按 **Ctrl+Alt+T**（默认，可在设置中录制新的组合键）
3. 翻译卡片出现在鼠标旁，点击同义词可在卡片内继续跳查
4. 按 **Ctrl+Alt+I**（或托盘菜单「输入翻译」）唤出输入窗口，打字/粘贴后 `Ctrl+Enter` 译为英文
5. 卡片点「对比各源」可一次查看各勾选源的译文（划词、截图卡片都支持）；输入翻译窗口默认就按源逐行列出
6. 托盘菜单或卡片右上角 ⚙ 打开设置

翻译源配置：

- 主翻译内置，无需任何配置
- 额外源（设置 → 翻译 → 源配置，左侧菜单切换源）：
  - 「百度翻译」：填入 [百度翻译开放平台](https://fanyi-api.baidu.com/) 的 APP ID / 密钥
  - 「腾讯翻译」：填入 [腾讯云访问管理 API 密钥](https://console.cloud.tencent.com/cam/capi) 的 SecretId / SecretKey（机器翻译每月有免费额度）
  - 填好后在「翻译源」里勾选，即出现在划词卡片的「对比各源」里
- 配置保存于 `%APPDATA%\com.wordpeek.desktop\config.json`（macOS 为 `~/Library/Application Support/com.wordpeek.desktop/config.json`，密钥为明文，请勿外传此文件）

## 原理

1. Rust 通过 `global-shortcut` 插件注册全局快捷键
2. 触发时：备份剪贴板 → 写入哨兵值 → 模拟 Ctrl+C（macOS 为 Cmd+C）→ 读取剪贴板 → 还原剪贴板
3. `GetCursorPos` 取鼠标坐标（macOS 为 `NSEvent.mouseLocation`），emit 事件给前端
4. React 收到事件后：移动弹窗到鼠标旁 → show → Rust 端请求翻译接口 → 渲染词典卡片

（翻译请求全部由 Rust 端发出，避免 webview CORS 限制，API 密钥不进前端。）

## 目录

```
src/components/Popup.tsx         # 划词/输入弹窗 UI + 事件监听
src/components/InputPage.tsx     # 输入翻译窗口（?page=input）
src/components/ScreenshotPage.tsx  # 截图遮罩框选（?page=screenshot）
src/components/SettingsPage.tsx  # 设置窗口
src-tauri/src/main.rs            # 全局快捷键、截图会话、托盘
src-tauri/src/platform/          # 平台差异：抓屏/鼠标坐标/剪贴板注入/权限
src-tauri/src/platform/windows.rs  # GDI BitBlt 抓屏、GetCursorPos、权限恒为已授
src-tauri/src/platform/macos.rs    # CoreGraphics 抓屏、NSEvent、AXIsProcessTrusted/CG 权限
src-tauri/src/capture.rs         # 纯函数：裁剪、缩放、PNG 编码
src-tauri/src/ocr.rs             # OCR 实现：Windows WinRT / macOS Vision + 共用图像预处理
src-tauri/src/translate.rs       # 翻译源（有道/必应/百度/腾讯）、配置读写
src-tauri/tauri.conf.json        # 无边框/置顶/透明弹窗 + 独立设置窗口
src-tauri/tauri.macos.conf.json  # macOS 打包覆盖：app+dmg、icon.icns、ad-hoc 签名、不产出更新签名
```

## 许可证

[MIT](./LICENSE)

## 改名记录

原名 pot-mini，为避免与 [pot-desktop](https://github.com/pot-app/pot-desktop) 项目产生混淆，已更名为 Wordpeek（本项目为独立实现）。首次启动会自动把旧目录（`%APPDATA%\com.potmini.app`）的配置迁移到新目录。

## 截图翻译

按 `Ctrl+Alt+S`(可在设置中修改)框选屏幕任意区域:

1. 全屏遮罩出现并冻结当前画面，拖拽框选，ESC 或点击窗外取消
2. 系统 OCR 本地识别文字(免 Key、离线、数据不出本机)：Windows 为 WinRT 引擎，macOS 为 Vision 框架
3. 识别结果自动翻译，弹出双语卡片，支持一键复制原文/译文

**OCR 语言**:设置 → 截图翻译 → 识别语言(跟随系统语言 / 中文优先 / 英文优先)。Windows 语言包随系统安装(设置 → 时间和语言 → 语言和区域)；macOS 语言由 Vision 内在支持，无需下载。

**注意**:单词与整句均免 Key(有道词条 + 必应整句);百度翻译密钥为可选项。

**局限**：仅支持 Windows 与 macOS；混合 DPI 多屏场景尽力而为（macOS 按每屏 backingScaleFactor 换算物理像素）。OCR 由系统引擎完成（离线、免 Key），已做小图放大、深底反色、分位拉伸与局部自适应二值化、Windows 下多语言引擎按质量打分择优；但艺术字体、手写体、严重模糊或强烈反光的图片仍可能识别不准。

| 功能 | 默认快捷键 |
|---|---|
| 划词翻译 | `Ctrl+Alt+T` |
| 截图翻译 | `Ctrl+Alt+S` |
| 输入翻译 | `Ctrl+Alt+I` |

macOS 上这三组默认键对应 `Control+Option+T/S/I`（`Ctrl`→Control、`Alt`→Option）；在设置里录制新组合键时也可以用 `Cmd`。所有快捷键都在托盘菜单 →「设置」→「快捷键」里改。
