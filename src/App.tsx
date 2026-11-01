import Popup from "./components/Popup";
import InputPage from "./components/InputPage";
import SettingsPage from "./components/SettingsPage";
import ScreenshotPage from "./components/ScreenshotPage";
import SelectionIcon from "./components/SelectionIcon";

export default function App() {
  // 设置/输入/截图窗口加载同一前端页面，用 URL 参数区分
  const page = new URLSearchParams(window.location.search).get("page");
  if (page === "settings") return <SettingsPage />;
  if (page === "input") return <InputPage />;
  if (page === "screenshot") return <ScreenshotPage />;
  if (page === "icon") return <SelectionIcon />;
  return <Popup />;
}
