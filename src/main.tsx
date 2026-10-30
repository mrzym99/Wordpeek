import React from "react";
import ReactDOM from "react-dom/client";
import { invoke } from "@tauri-apps/api/core";
import App from "./App";
import { applyTheme, watchTheme, type Theme } from "./lib/theme";
import "./style.css";

// 四个窗口共用这一份页面：主题在这里统一落地，页面组件不必各自处理
// 设置/输入窗口在 macOS 上自绘标题栏（Rust 侧 TitleBarStyle::Overlay），CSS 靠这个标记决定要不要让出头部
if (navigator.userAgent.includes("Mac")) document.documentElement.dataset.os = "mac";
let theme: Theme = "system";
const syncTheme = () =>
  invoke<{ theme?: Theme }>("get_config")
    .then((cfg) => {
      theme = cfg.theme ?? "system";
      applyTheme(theme);
    })
    .catch(() => applyTheme(theme));
syncTheme();
// 设置页改了主题后，已存在的弹窗窗口下次获得焦点时跟上
window.addEventListener("focus", syncTheme);
watchTheme(() => theme);

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>
);
