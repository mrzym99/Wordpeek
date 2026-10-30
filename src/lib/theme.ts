/** 外观主题：system 跟随系统，light/dark 强制。DOM 上只落 <html data-theme="light|dark">，
    样式表里就是两套 token，没有第三套 */
export type Theme = "system" | "light" | "dark";

const darkQuery = window.matchMedia("(prefers-color-scheme: dark)");

export function applyTheme(theme: Theme) {
  const dark = theme === "system" ? darkQuery.matches : theme === "dark";
  document.documentElement.dataset.theme = dark ? "dark" : "light";
  // index.html 的内联脚本读这个值，深色系统下首屏不会先闪一下浅色
  localStorage.setItem("wordpeek-theme", theme);
}

/** 系统外观变化时按当前设置重算（get 返回设置页此刻选中的值） */
export function watchTheme(get: () => Theme) {
  const onChange = () => applyTheme(get());
  darkQuery.addEventListener("change", onChange);
  return () => darkQuery.removeEventListener("change", onChange);
}
