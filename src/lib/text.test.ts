import test from "node:test";
import assert from "node:assert/strict";
import { isSentenceText } from "./text.ts";
import { splitSense } from "./senses.ts";

test("isSentenceText: 单词 vs 句子", () => {
  assert.equal(isSentenceText("hello"), false);
  assert.equal(isSentenceText("hello world"), true);
  assert.equal(isSentenceText("你好！"), true);
  // 中文超过 2 个字当句子（中文没空格可看）
  assert.equal(isSentenceText("错误"), false);
  assert.equal(isSentenceText("错误码解析"), true);
});

test("splitSense: 词性和释义拆分", () => {
  assert.deepEqual(splitSense("n. 错误，差错"), { pos: "n.", text: "错误，差错" });
  assert.deepEqual(splitSense("错误，差错"), { pos: "", text: "错误，差错" });
});
