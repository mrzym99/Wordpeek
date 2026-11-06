/* 下载按钮：直接下载 GitHub 最新 Release 里的安装包。
   Release 产物名带版本号（Wordpeek_2.2.0_aarch64.dmg），没法写死
   releases/latest/download/ 的固定路径，所以用 GitHub API 查 latest 的 assets，
   再把按钮 href 指向真实资产；接口失败（离线 / 限流）时保留指向 Releases 页的兜底链接。 */

(function () {
  "use strict";

  var API = "https://api.github.com/repos/mrzym99/Wordpeek/releases/latest";
  var CACHE_KEY = "wordpeek:latest-release";
  var CACHE_MS = 30 * 60 * 1000; // 30 分钟内复用，避免每次刷新都打 API

  var buttons = Array.prototype.slice.call(document.querySelectorAll("[data-download]"));
  if (!buttons.length) return;

  function pickAsset(assets, kind) {
    var exact = kind === "mac" ? /(_aarch64|arm64)\.dmg$/i : /-setup\.exe$/i;
    var loose = kind === "mac" ? /\.dmg$/i : /\.exe$/i;
    var hit = assets.filter(function (a) {
      return exact.test(a.name || "");
    })[0];
    if (hit) return hit;
    return (
      assets.filter(function (a) {
        return loose.test(a.name || "");
      })[0] || null
    );
  }

  function apply(release) {
    if (!release) return;
    var assets = release.assets || [];
    buttons.forEach(function (btn) {
      var asset = pickAsset(assets, btn.getAttribute("data-download"));
      if (!asset || !asset.browser_download_url) return;
      // 只接受本仓库的 release 资产，接口异常时不会把用户带去别处
      if (!/^https:\/\/github\.com\/mrzym99\/Wordpeek\/releases\/download\//.test(asset.browser_download_url)) {
        return;
      }
      btn.href = asset.browser_download_url;
    });
    var ver = document.getElementById("dlVersion");
    if (ver && release.tag_name) {
      ver.textContent = release.tag_name;
      ver.hidden = false;
    }
  }

  function readCache() {
    try {
      var saved = JSON.parse(localStorage.getItem(CACHE_KEY) || "null");
      if (!saved || !saved.at || Date.now() - saved.at > CACHE_MS) return null;
      return saved.release;
    } catch (e) {
      return null;
    }
  }

  function writeCache(release) {
    try {
      localStorage.setItem(
        CACHE_KEY,
        JSON.stringify({
          at: Date.now(),
          release: {
            tag_name: release.tag_name,
            assets: (release.assets || []).map(function (a) {
              return { name: a.name, browser_download_url: a.browser_download_url };
            }),
          },
        })
      );
    } catch (e) {
      /* 隐私模式下 localStorage 不可用：忽略缓存即可 */
    }
  }

  // Windows 访客把 Windows 按钮提为主按钮（其余情况默认 macOS 为主）
  if (/Windows/i.test(navigator.userAgent || "")) {
    buttons.forEach(function (btn) {
      var isWin = btn.getAttribute("data-download") === "win";
      btn.classList.toggle("btn-primary", isWin);
      btn.classList.toggle("btn-ghost", !isWin);
    });
  }

  var cached = readCache();
  if (cached) {
    apply(cached);
    return;
  }

  fetch(API, { headers: { Accept: "application/vnd.github+json" } })
    .then(function (res) {
      if (!res.ok) throw new Error("HTTP " + res.status);
      return res.json();
    })
    .then(function (release) {
      apply(release);
      writeCache(release);
    })
    .catch(function () {
      /* 网络/限流失败：按钮继续指向 Releases 页，不会变成死链 */
    });
})();
