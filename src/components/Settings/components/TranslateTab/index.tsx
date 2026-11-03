import { SOURCE_ITEMS, type ConfigForm } from "../../../../hooks/useConfigForm";
import s from "../../index.module.css";

/** 「翻译」页：翻译源勾选+拖拽排序（列表）/ 源配置（密钥表单），横向 tab 切换 */
export default function TranslateTab({ f }: { f: ConfigForm }) {
  return (
    <>
      {/* 翻译源两个子页：列表（勾选+拖拽排序）/ 源配置，横向 tab 切换 */}
      <div className={s.subTabs}>
        <button className={s.subTab + (f.srcTab === "list" ? " active" : "")} onClick={() => f.setSrcTab("list")}>
          翻译源
        </button>
        <button className={s.subTab + (f.srcTab === "config" ? " active" : "")} onClick={() => f.setSrcTab("config")}>
          源配置
        </button>
      </div>
      {f.srcTab === "list" ? <SourceList f={f} /> : <SourceConfig f={f} />}
    </>
  );
}

function SourceList({ f }: { f: ConfigForm }) {
  return (
    <>
      <div className={s.groupHeading}>翻译源（至少保留一个）</div>
      <div className={s.sourceList}>
        <div className={s.sourceItem + (f.builtin ? "" : " " + s.off)}>
          <label className={s.switch}>
            <input type="checkbox" checked={f.builtin} onChange={f.toggleBuiltin} />
            默认源：有道词典 / 必应翻译（内置，免 Key）
          </label>
        </div>
        {f.sources.map((id) => {
          const item = SOURCE_ITEMS.find((it) => it.id === id);
          if (!item) return null;
          return (
            <div
              key={id}
              className={s.sourceItem}
              draggable
              onDragStart={(e) => {
                // 窗口已关 dragDropEnabled；setData 不调 WebView2 里 drop 不触发
                e.dataTransfer.setData("text/plain", id);
                e.dataTransfer.effectAllowed = "move";
                f.setDragId(id);
              }}
              onDragOver={(e) => e.preventDefault()}
              onDrop={(e) => {
                e.preventDefault();
                f.moveSource(f.dragId, id);
                f.setDragId("");
              }}
              onDragEnd={() => f.setDragId("")}
            >
              <span className={s.dragHandle} title="拖拽调整顺序">
                ⠿
              </span>
              <label className={s.switch}>
                <input
                  type="checkbox"
                  checked
                  disabled={item.needKey && !f.ready(item.id)}
                  title={item.needKey && !f.ready(item.id) ? "密钥未配置，请先在「源配置」里填写" : undefined}
                  onChange={() => f.toggleSource(id)}
                />
                {item.label}
              </label>
            </div>
          );
        })}
        {SOURCE_ITEMS.filter((it) => !f.sources.includes(it.id)).map((it) => (
          <div key={it.id} className={s.sourceItem + " " + s.off}>
            <span className={s.dragHandle} />
            <label className={s.switch}>
              <input
                type="checkbox"
                checked={false}
                disabled={it.needKey && !f.ready(it.id)}
                title={it.needKey && !f.ready(it.id) ? "密钥未配置，请先在「源配置」里填写" : undefined}
                onChange={() => f.toggleSource(it.id)}
              />
              {it.label}
            </label>
          </div>
        ))}
      </div>
      <div className="field-hint">
        默认源免 Key：单词用有道、句子用必应，失败自动降级；额外源在「源配置」填好密钥后勾选
      </div>
    </>
  );
}

function SourceConfig({ f }: { f: ConfigForm }) {
  return (
    // 源配置：左侧菜单选源，右侧编辑对应密钥
    <div className={s.configSplit}>
      <div className={s.configMenu}>
        {SOURCE_ITEMS.map((it) => (
          <button
            key={it.id}
            type="button"
            className={s.configMenuItem + (f.cfgSrc === it.id ? " active" : "")}
            onClick={() => f.setCfgSrc(it.id as "baidu" | "tencent")}
          >
            {it.label.replace("（需密钥）", "")}
            {f.ready(it.id) && <span className={s.configDot} title="已配置" />}
          </button>
        ))}
      </div>
      <div className={s.configForm}>
        {f.cfgSrc === "baidu" ? (
          <>
            <div className={s.groupHeading}>百度翻译密钥（可选）</div>
            <label className="field">
              APP ID
              <input
                value={f.appid}
                placeholder="百度翻译开放平台 appid"
                onChange={(e) => f.setAppid(e.target.value)}
              />
            </label>
            <label className="field">
              密钥
              <input
                type="password"
                value={f.secret}
                placeholder="百度翻译开放平台密钥"
                onChange={(e) => f.setSecret(e.target.value)}
              />
            </label>
          </>
        ) : (
          <>
            <div className={s.groupHeading}>腾讯云密钥（可选）</div>
            <label className="field">
              SecretId
              <input
                value={f.tcId}
                placeholder="腾讯云访问管理 API 密钥 SecretId"
                onChange={(e) => f.setTcId(e.target.value)}
              />
            </label>
            <label className="field">
              SecretKey
              <input
                type="password"
                value={f.tcKey}
                placeholder="腾讯云访问管理 API 密钥 SecretKey"
                onChange={(e) => f.setTcKey(e.target.value)}
              />
            </label>
          </>
        )}
      </div>
    </div>
  );
}
