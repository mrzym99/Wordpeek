/* 划词演示：纯前端模拟「选中单词 → 浮出图标 → 展开词典卡」的交互。
   真实应用还会接系统选区与翻译源，这里用内置示例词典保证离线可用。
   段落里的词在启动时逐个包成 .demo-word，所以「任意单词」都能查，不是只挑好的几个。 */

(function () {
  "use strict";

  /* 词典条目：[音标, [[词性, 释义], ...]]。覆盖演示段落里的每个词，避免选到就落空 */
  var DICT = {
    a: ["/ə/", [["art.", "一（个）"]]],
    alone: ["/əˈləʊn/", [["adv.", "单独地；仅仅"]]],
    an: ["/ən/", [["art.", "一（个）"]]],
    and: ["/ænd/", [["conj.", "和，并且"]]],
    anyone: ["/ˈeniwʌn/", [["pron.", "任何人"]]],
    becomes: ["/bɪˈkʌmz/", [["v.", "变成（become 的第三人称单数）"]]],
    can: ["/kæn/", [["modal v.", "能，可以"]]],
    cannot: ["/ˈkænɒt/", [["v.", "不能"]]],
    carry: ["/ˈkæri/", [["v.", "携带；承载；传达"]]],
    chore: ["/tʃɔː(r)/", [["n.", "家务；乏味的例行公事"]]],
    context: ["/ˈkɒntekst/", [["n.", "上下文；语境"]]],
    dictionary: ["/ˈdɪkʃənəri/", [["n.", "词典，字典"]]],
    eloquent: ["/ˈeləkwənt/", [["adj.", "雄辩的，有说服力的"], ["adj.", "富于表现力的"]]],
    english: ["/ˈɪŋɡlɪʃ/", [["n.", "英语"], ["adj.", "英国的；英语的"]]],
    ephemeral: ["/ɪˈfemərəl/", [["adj.", "短暂的，转瞬即逝的"]]],
    feels: ["/fiːlz/", [["v.", "感觉（feel 的第三人称单数）"]]],
    how: ["/haʊ/", [["adv.", "怎样，如何"]]],
    in: ["/ɪn/", [["prep.", "在……里；在……期间"]]],
    into: ["/ˈɪntuː/", [["prep.", "进入……里面；成为"]]],
    learning: ["/ˈlɜːnɪŋ/", [["n.", "学习；学问"]]],
    let: ["/let/", [["v.", "让，允许"]]],
    meet: ["/miːt/", [["v.", "遇见；使结识"]]],
    memory: ["/ˈmeməri/", [["n.", "记忆；回忆"]]],
    notice: ["/ˈnəʊtɪs/", [["v.", "注意到，察觉到"], ["n.", "通知；告示"]]],
    nuance: ["/ˈnjuːɑːns/", [["n.", "细微差别，微妙之处"], ["v.", "使有细微差别"]]],
    practice: ["/ˈpræktɪs/", [["n.", "练习；实践"]]],
    rather: ["/ˈrɑːðə/", [["adv.", "宁可；相当，颇"]]],
    resilient: ["/rɪˈzɪliənt/", [["adj.", "有弹性的；能迅速恢复的"], ["adj.", "有韧性的，适应力强的"]]],
    sentence: ["/ˈsentəns/", [["n.", "句子；判决"]]],
    serendipity: ["/ˌserənˈdɪpəti/", [["n.", "意外发现珍奇事物的本领；机缘巧合"]]],
    settle: ["/ˈsetl/", [["v.", "安定；定居；解决"]]],
    some: ["/sʌm/", [["adj.", "一些的；某个"], ["pron.", "一些"]]],
    speak: ["/spiːk/", [["v.", "说话，讲话"]]],
    teach: ["/tiːtʃ/", [["v.", "教，讲授"]]],
    than: ["/ðæn/", [["conj.", "比"]]],
    that: ["/ðæt/", [["pron.", "那个"], ["conj.", "引导从句"]]],
    tranquil: ["/ˈtræŋkwɪl/", [["adj.", "平静的，安宁的"]]],
    ubiquitous: ["/juːˈbɪkwɪtəs/", [["adj.", "无所不在的；普遍存在的"], ["adj.", "随处可见的，到处都有的"]]],
    wander: ["/ˈwɒndə(r)/", [["v.", "漫步，闲逛"], ["v.", "走神，思绪飘移"]]],
    way: ["/weɪ/", [["n.", "方式，方法；道路"]]],
    with: ["/wɪð/", [["prep.", "和……一起；用"]]],
    words: ["/wɜːdz/", [["n.", "单词；话语（word 的复数）"]]],
  };

  var panel = document.getElementById("demoPanel");
  var text = document.getElementById("demoText");
  var floatBtn = document.getElementById("demoFloat");
  var card = document.getElementById("demoCard");
  if (!panel || !text || !floatBtn || !card) return;

  var ICON_SIZE = 38;
  var GAP = 8; // 图标与选区的间距
  var CARD_GAP = 10; // 卡片与选区的间距
  var EDGE = 8; // 贴边留白

  var currentWord = "";
  var anchor = null; // 选区的视口矩形：图标与卡片都以它为锚
  var activeSpan = null;

  function clamp(v, min, max) {
    return Math.min(Math.max(v, min), Math.max(min, max));
  }

  function escapeHtml(s) {
    return String(s).replace(/[&<>"']/g, function (c) {
      return { "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c];
    });
  }

  /* ---- 把段落里的英文词逐个包成 span：整段任意单词都能点 ---- */
  function wrapWords(root) {
    var walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT, null);
    var nodes = [];
    var node;
    while ((node = walker.nextNode())) nodes.push(node);
    nodes.forEach(function (n) {
      var raw = n.nodeValue;
      if (!/[A-Za-z]/.test(raw)) return;
      var frag = document.createDocumentFragment();
      var re = /[A-Za-z]+(?:['’\-][A-Za-z]+)*/g;
      var last = 0;
      var m;
      while ((m = re.exec(raw))) {
        if (m.index > last) frag.appendChild(document.createTextNode(raw.slice(last, m.index)));
        var span = document.createElement("span");
        span.className = "demo-word";
        span.textContent = m[0];
        frag.appendChild(span);
        last = m.index + m[0].length;
      }
      if (last < raw.length) frag.appendChild(document.createTextNode(raw.slice(last)));
      n.parentNode.replaceChild(frag, n);
    });
  }

  /** 从选区里取出一个英文词；整句/空白返回 null */
  function wordFromSelection(sel) {
    if (!sel || sel.isCollapsed) return null;
    var raw = sel.toString().trim();
    if (!raw || raw.length > 30 || /\s/.test(raw)) return null;
    var w = raw.replace(/^[^\p{L}\p{N}'’‑-]+|[^\p{L}\p{N}'’‑-]+$/gu, "");
    return w || null;
  }

  /** 当前选区若落在演示段落里的一个单词上，返回 { word, rect } */
  function selectionHit() {
    var sel = window.getSelection();
    if (!sel || sel.isCollapsed || !sel.rangeCount) return null;
    if (!text.contains(sel.anchorNode) || !text.contains(sel.focusNode)) return null;
    var word = wordFromSelection(sel);
    if (!word) return null;
    var rect = sel.getRangeAt(0).getBoundingClientRect();
    if (!rect || (!rect.width && !rect.height)) return null;
    return { word: word, rect: rect };
  }

  function clearActive() {
    if (activeSpan) {
      activeSpan.classList.remove("active");
      activeSpan = null;
    }
  }

  function markActive(word) {
    clearActive();
    var spans = text.querySelectorAll(".demo-word");
    for (var i = 0; i < spans.length; i++) {
      if (spans[i].textContent.toLowerCase() === word.toLowerCase()) {
        spans[i].classList.add("active");
        activeSpan = spans[i];
        break;
      }
    }
  }

  function hideFloat() {
    floatBtn.hidden = true;
  }

  function hideCard() {
    card.hidden = true;
    card.innerHTML = "";
  }

  function hideAll() {
    hideFloat();
    hideCard();
    clearActive();
    currentWord = "";
    anchor = null;
  }

  /** 在选区右下角浮出图标；坐标是视口坐标（fixed） */
  function showFloat(word, rect) {
    currentWord = word;
    anchor = rect;
    hideCard();
    floatBtn.hidden = false;
    var left = clamp(rect.right + GAP, EDGE, window.innerWidth - ICON_SIZE - EDGE);
    var top = clamp(rect.bottom + GAP, EDGE, window.innerHeight - ICON_SIZE - EDGE);
    floatBtn.style.left = left + "px";
    floatBtn.style.top = top + "px";
    // hidden 切回来会重播入场动画；清一次内联 animation 保证连选连播
    floatBtn.style.animation = "none";
    void floatBtn.offsetWidth;
    floatBtn.style.animation = "";
  }

  function renderCard(word) {
    var entry = DICT[word.toLowerCase()];
    var html =
      '<div class="dict-head"><div class="dict-brand">Wordpeek<span class="beta">beta</span></div>' +
      '<button class="dict-close" type="button" aria-label="关闭词典卡">' +
      '<svg viewBox="0 0 16 16" aria-hidden="true"><path d="M4 4l8 8M12 4l-8 8"/></svg>' +
      "</button></div>";
    html += '<div class="dict-word">' + escapeHtml(word);
    if (entry && entry[0]) html += '<span class="dict-phon">' + escapeHtml(entry[0]) + "</span>";
    html += "</div>";
    if (entry) {
      html += '<ol class="dict-senses">';
      entry[1].forEach(function (s) {
        html += "<li><i>" + escapeHtml(s[0]) + "</i> " + escapeHtml(s[1]) + "</li>";
      });
      html += "</ol>";
    } else {
      html +=
        '<div class="dict-empty">演示词典暂未收录这个词。<br />实际应用会调用有道 / 必应实时翻译。</div>';
    }
    html += '<div class="dict-foot">演示词典 · 实际翻译由有道 / 必应提供</div>';
    card.innerHTML = html;
  }

  /** 展开词典卡：贴着选区展开（不是贴着图标），下方放不下就翻到上方 */
  function openCard() {
    if (!currentWord || !anchor) return;
    renderCard(currentWord);
    card.hidden = false;
    card.style.left = "0px";
    card.style.top = "0px";
    var cw = card.offsetWidth;
    var ch = card.offsetHeight;
    var left = clamp(anchor.left, EDGE, window.innerWidth - cw - EDGE);
    var top = anchor.bottom + CARD_GAP;
    if (top + ch > window.innerHeight - EDGE) top = anchor.top - ch - CARD_GAP;
    top = clamp(top, EDGE, window.innerHeight - ch - EDGE);
    card.style.left = left + "px";
    card.style.top = top + "px";
    hideFloat();
  }

  /* ---- 选中文字 → 浮出图标（监听 document：拖到段落外松手也能收到） ---- */
  document.addEventListener("mouseup", function () {
    var sel = window.getSelection();
    if (!sel || sel.isCollapsed) return;
    if (!text.contains(sel.anchorNode) || !text.contains(sel.focusNode)) return;
    var hit = selectionHit();
    if (!hit) {
      // 选中的不是单个单词（整句/空白）：收起，不在错误的锚点上弹
      hideAll();
      return;
    }
    markActive(hit.word);
    showFloat(hit.word, hit.rect);
  });

  /* ---- 直接点单词：轻点也能弹图标（触屏更顺手） ---- */
  text.addEventListener("click", function (e) {
    var span = e.target.closest(".demo-word");
    if (!span) return;
    var sel = window.getSelection();
    if (sel && !sel.isCollapsed && sel.toString().trim()) return; // 交给选区流程
    markActive(span.textContent);
    showFloat(span.textContent, span.getBoundingClientRect());
  });

  /* ---- 图标 → 词典卡 ---- */
  floatBtn.addEventListener("click", function (e) {
    e.preventDefault();
    openCard();
  });

  /* ---- 卡片上的关闭按钮 ---- */
  card.addEventListener("click", function (e) {
    if (e.target.closest(".dict-close")) hideAll();
  });

  /* ---- 快捷词条：一键看效果，也是键盘可达入口 ---- */
  var chips = panel.querySelectorAll(".demo-chip");
  for (var i = 0; i < chips.length; i++) {
    (function (chip) {
      chip.addEventListener("click", function () {
        var word = chip.getAttribute("data-word");
        if (!word) return;
        currentWord = word;
        anchor = chip.getBoundingClientRect();
        markActive(word);
        openCard();
      });
    })(chips[i]);
  }

  /* ---- Alt/⌥ + Z 直接展开（与真实应用的热键一致）---- */
  document.addEventListener("keydown", function (e) {
    if (e.key === "Escape") {
      hideAll();
      return;
    }
    if (!e.altKey) return;
    // macOS 的 Option+Z 会产出 key="Ω"，用物理键位判断更稳
    if (e.code !== "KeyZ" && e.key.toLowerCase() !== "z") return;
    var hit = selectionHit();
    if (hit) {
      markActive(hit.word);
      currentWord = hit.word;
      anchor = hit.rect;
    }
    if (!currentWord || !anchor) return;
    e.preventDefault();
    openCard();
  });

  /* ---- 点外面 / 滚动 / 缩放：收起 ---- */
  document.addEventListener("pointerdown", function (e) {
    if (e.target.closest("#demoFloat, #demoCard, .demo-word, .demo-chip")) return;
    hideAll();
  });
  window.addEventListener("scroll", hideAll, { passive: true });
  window.addEventListener("resize", hideAll);

  wrapWords(text);
})();
