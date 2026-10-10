import Popup from "./components/Popup";
import SettingsPage from "./components/SettingsPage";
import ScreenshotPage from "./components/ScreenshotPage";

export default function App() {
  // 设置/截图窗口加载同一前端页面，用 URL 参数区分
  const page = new URLSearchParams(window.location.search).get("page");
  if (page === "settings") return <SettingsPage />;
  if (page === "screenshot") return <ScreenshotPage />;
  return <Popup />;
}
