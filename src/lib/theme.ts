/** 外观主题：system 跟随系统，light/dark 强制。DOM 上只落 <html data-theme="light|dark">，
    样式表里就是两套 token，没有第三套 */
import { getCurrentWindow } from "@tauri-apps/api/window";
import { listen } from "@tauri-apps/api/event";

export type Theme = "system" | "light" | "dark";
/** Tauri 原生窗口的主题只有亮暗两态 */
type NativeTheme = "light" | "dark";

const darkQuery = window.matchMedia("(prefers-color-scheme: dark)");
/** 本窗口此刻生效的设置档，跟随判断以它为准（main.tsx 的模块变量不会随设置页点选更新） */
let applied: Theme = "system";

function setDomTheme(dark: boolean) {
  document.documentElement.dataset.theme = dark ? "dark" : "light";
}

export function applyTheme(theme: Theme) {
  applied = theme;
  const dark = theme === "system" ? darkQuery.matches : theme === "dark";
  setDomTheme(dark);
  // index.html 的内联脚本读这个值，深色系统下首屏不会先闪一下浅色
  localStorage.setItem("wordpeek-theme", theme);
  // 只有强制亮/暗才把原生标题栏钉在具体值上；跟随系统必须传 null。
  // Windows 上传具体值会让 tao 记下 preferred_theme=Some(..)，之后系统深浅色
  // 切换的 WM_SETTINGCHANGE 被直接吞掉，WebView2 的 prefers-color-scheme 冻死
  try {
    getCurrentWindow()
      .setTheme(theme === "system" ? null : dark ? "dark" : "light")
      .then(() => {
        // 从强制档切回 system 时上面的 dark 可能还读着旧钉住值；setTheme 生效
        // 后媒体查询已被原生刷新，补对齐一次，不必等 change 事件
        if (theme === "system") setDomTheme(darkQuery.matches);
      })
      .catch(() => {});
  } catch {
    // 非 Tauri 环境（纯前端调试）忽略
  }
}

/** 系统外观变化时重新落地。Windows 的 WebView2 只翻转 prefers-color-scheme 状态、
    不派发 change 回调，以 Rust 在原生 ThemeChanged 时广播的解析值为准；
    标题栏重绘也由 Rust 侧统一处理。macOS/纯浏览器仍走媒体查询监听兜底 */
export function watchTheme() {
  const onMediaChange = () => applyTheme(applied);
  darkQuery.addEventListener("change", onMediaChange);

  let unlisten: (() => void) | undefined;
  let disposed = false;
  listen<NativeTheme>("system-theme-changed", (e) => {
    if (applied === "system") setDomTheme(e.payload === "dark");
  })
    .then((u) => (disposed ? u() : (unlisten = u)))
    .catch(() => {});

  return () => {
    disposed = true;
    darkQuery.removeEventListener("change", onMediaChange);
    unlisten?.();
  };
}
