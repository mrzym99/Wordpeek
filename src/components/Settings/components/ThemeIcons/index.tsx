import { applyTheme, type Theme } from "../../../../lib/theme";
import { saveTheme } from "../../../../lib/config";
import s from "../../index.module.css";

/** 主题三档（顺序即显示顺序），点图标即时生效：设置窗口自己先变，其它窗口下次聚焦时跟上 */
const THEMES: { id: Theme; label: string; icon: React.ReactNode }[] = [
  {
    id: "system",
    label: "跟随系统",
    icon: (
      <>
        <rect x="2.25" y="3.25" width="11.5" height="8" rx="1.5" />
        <path d="M6.5 14.25h3M8 11.25v3" />
      </>
    ),
  },
  {
    id: "light",
    label: "亮色",
    icon: (
      <>
        <circle cx="8" cy="8" r="3.25" />
        <path d="M8 1.5v1.5M8 13v1.5M1.5 8H3M13 8h1.5M3.4 3.4l1.05 1.05M11.55 11.55l1.05 1.05M12.6 3.4l-1.05 1.05M4.45 11.55 3.4 12.6" />
      </>
    ),
  },
  {
    id: "dark",
    label: "暗色",
    icon: <path d="M13.5 9.6A5.75 5.75 0 0 1 6.4 2.5a5.75 5.75 0 1 0 7.1 7.1Z" />,
  },
];

export default function ThemeIcons({ value, onChange }: { value: Theme; onChange: (t: Theme) => void }) {
  return (
    <div className={s.themeIcons}>
      {THEMES.map((it) => (
        <button
          key={it.id}
          type="button"
          className={s.themeIcon + (value === it.id ? " active" : "")}
          title={it.label}
          aria-label={it.label}
          onClick={() => {
            onChange(it.id);
            applyTheme(it.id);
            saveTheme(it.id).catch(() => {});
          }}
        >
          <svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" aria-hidden>
            {it.icon}
          </svg>
        </button>
      ))}
    </div>
  );
}