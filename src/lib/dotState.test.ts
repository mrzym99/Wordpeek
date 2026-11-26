import { test } from "node:test";
import assert from "node:assert/strict";
import { resolveState } from "./dotState.ts";

test("near 边沿驱动三态：false=小点 true=展开胶囊", () => {
  assert.equal(resolveState(false), "dot");
  assert.equal(resolveState(true), "expanded");
});

test("初始为小点态（Rust 边沿触发保证停在旁边不会立即展开）", () => {
  // Rust 首发事件恒为 {near:false}，前端不自行推断 near
  assert.equal(resolveState(false), "dot");
});
