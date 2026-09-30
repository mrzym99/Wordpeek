# Wordpeek — 最小划词翻译（Tauri + React）

在任意窗口选中单词，按下快捷键，鼠标旁弹出翻译卡片；点击窗外自动隐藏。

![](https://img.shields.io/badge/platform-Windows-blue) ![](https://img.shields.io/badge/license-MIT-green)

## 功能

- **划词翻译**：全局快捷键抓取选中文本，鼠标位置弹出翻译卡片
- **丰富的词典卡片**：音标（美/英）、词性、多释义、词形变化、同义词（可点击跳查）、考试标签
- **多翻译源**：有道词典（免 Key）/ 百度翻译 / 微软 Azure 翻译；主源失败自动降级
- **可配置**：独立设置窗口——翻译源、API 密钥、快捷键（支持录制组合键）
- **系统托盘**：常驻后台，托盘菜单打开设置或退出

## 下载安装

前往 [Releases](../../releases) 下载 `Wordpeek_x.x.x_x64-setup.exe` 安装，或下载便携版 `wordpeek.exe` 直接运行。

> 未做代码签名，首次运行 Windows SmartScreen 可能拦截，选择"仍要运行"即可。

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
| 微软翻译 | [Azure](https://portal.azure.com) 创建"翻译器"资源获取密钥（免费 F0 档每月 200 万字符） |

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
src-tauri/src/translate.rs  # 翻译源（有道/百度/微软）、配置读写
src-tauri/tauri.conf.json   # 无边框/置顶/透明弹窗 + 独立设置窗口
```

## 许可证

[MIT](./LICENSE)

## 改名记录

原名 pot-mini，为避免与 [pot-desktop](https://github.com/pot-app/pot-desktop) 项目产生混淆，已更名为 Wordpeek（本项目为独立实现）。首次启动会自动把旧目录（`%APPDATA%\com.potmini.app`）的配置迁移到新目录。
