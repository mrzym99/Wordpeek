import test from "node:test";
import assert from "node:assert/strict";
import { externalClickAfterShow, pointInRect, shouldDismiss } from "./cardDismiss.ts";

const rect = { pos: { x: 100, y: 200 }, size: { width: 400, height: 300 } };

test("shouldDismiss: 鼠标在卡片内永不消失", () => {
  assert.equal(
    shouldDismiss({ cursorInCard: true, everFocused: true, focused: false, externalClick: true }),
    false,
  );
});

test("shouldDismiss: 模式一（拿到过焦点又失去）blur 语义", () => {
  const base = { cursorInCard: false, externalClick: false };
  // 失焦即收
  assert.equal(shouldDismiss({ ...base, everFocused: true, focused: false }), true);
  // 还有焦点不收
  assert.equal(shouldDismiss({ ...base, everFocused: true, focused: true }), false);
});

test("shouldDismiss: 模式二（焦点从未立起来）全局点击语义", () => {
  const base = { cursorInCard: false, focused: false };
  // 点过卡片外才收
  assert.equal(shouldDismiss({ ...base, everFocused: false, externalClick: true }), true);
  // 还没点过外部就留着
  assert.equal(shouldDismiss({ ...base, everFocused: false, externalClick: false }), false);
});

test("externalClickAfterShow: 入场豁免与窗口内命中", () => {
  // shownAt=0（show 尚未发生，轮询不会跑；防御性）：show 前记录的按下也算外部
  assert.equal(externalClickAfterShow([150, 160, 1000], 0, false), true);
  assert.equal(externalClickAfterShow([150, 160, 500], 1000, false), false);
  // show 之后的按下才算（时间戳必须严格大于 shownAt）
  assert.equal(externalClickAfterShow([150, 160, 1001], 1000, false), true);
  assert.equal(externalClickAfterShow([150, 160, 1000], 1000, false), false);
  // 没有任何按下记录
  assert.equal(externalClickAfterShow(null, 1000, false), false);
  // 点在自己窗口矩形内不算外部（透明边距交给 DOM 判）
  assert.equal(externalClickAfterShow([150, 160, 1001], 1000, true), false);
});

test("pointInRect: 半开区间判定", () => {
  assert.equal(pointInRect(100, 200, rect.pos, rect.size), true);
  assert.equal(pointInRect(499, 499, rect.pos, rect.size), true);
  // 右/下边界外
  assert.equal(pointInRect(500, 250, rect.pos, rect.size), false);
  assert.equal(pointInRect(300, 500, rect.pos, rect.size), false);
});
