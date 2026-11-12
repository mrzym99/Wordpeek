import { SOURCE_ITEMS, type ConfigForm } from "../../../../hooks/useConfigForm";
import s from "../../index.module.css";

/** 「翻译」页：一个列表兼做启用、排序和密钥配置（点行展开表单），不再分子 tab */
export default function TranslateTab({ f }: { f: ConfigForm }) {
  // 已启用的按用户拖出来的顺序在前，未启用的按 SOURCE_ITEMS 原序跟在后面
  const order = [...f.sources, ...SOURCE_ITEMS.map((it) => it.id).filter((id) => !f.sources.includes(id))];

  return (
    <>
      <div className={s.accList}>
        {/* 内置默认源：免 Key，没有表单可展开，只给开关 */}
        <div className={s.accItem}>
          <div className={s.accHead}>
            <span className={s.dragHandle} />
            <span className={s.accTitle + " " + s.accStatic}>内置默认源（有道 / 必应，免 Key）</span>
            <label className={s.switch}>
              <input type="checkbox" checked={f.builtin} onChange={f.toggleBuiltin} />
            </label>
            <span className={s.accChevronEmpty} />
          </div>
        </div>

        {order.map((id) => {
          const item = SOURCE_ITEMS.find((it) => it.id === id);
          if (!item) return null;
          const enabled = f.sources.includes(id);
          const open = f.cfgSrc === id;
          const isReady = f.ready(id);
          // 密钥没填就不给开；已启用的永远能关（关掉不受此限）
          const blocked = item.needKey && !isReady && !enabled;
          const toggle = () => f.setCfgSrc(open ? "" : (id as "baidu" | "tencent"));

          return (
            <div
              key={id}
              className={s.accItem + (open ? " " + s.open : "") + (enabled ? "" : " " + s.off)}
              // 只有启用中的源可拖：禁用行拖进列表会被当成"插进 sources"而被动启用
              draggable={enabled}
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
              <div className={s.accHead}>
                <span className={s.dragHandle} title={enabled ? "拖拽调整顺序" : "启用后可拖拽排序"}>
                  ⠿
                </span>
                <button type="button" className={s.accTitle} onClick={toggle}>
                  {item.label.replace("（需密钥）", "")}
                </button>
                <label className={s.switch} title={blocked ? "密钥未填，填好后可启用" : undefined}>
                  <input
                    type="checkbox"
                    checked={enabled}
                    disabled={blocked}
                    onChange={() => f.toggleSource(id)}
                  />
                </label>
                <button type="button" className={s.accChevronBtn} onClick={toggle} aria-label="展开 / 收起">
                  <svg className={s.accChevron} width="14" height="14" viewBox="0 0 16 16" fill="none" aria-hidden>
                    <path
                      d="M6 3.5 10.5 8 6 12.5"
                      stroke="currentColor"
                      strokeWidth="1.5"
                      strokeLinecap="round"
                      strokeLinejoin="round"
                    />
                  </svg>
                </button>
              </div>

              {open && (
                <div className={s.accBody}>
                  {id === "baidu" ? (
                    <>
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
                  <button
                    className={s.saveBtn}
                    onClick={id === "baidu" ? f.saveBaiduKeys : f.saveTencentKeys}
                    disabled={f.loadFailed}
                  >
                    保存
                  </button>
                </div>
              )}
            </div>
          );
        })}
      </div>
    </>
  );
}