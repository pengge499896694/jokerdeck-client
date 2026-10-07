import { test } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import vm from "node:vm";

const source = fs.readFileSync(new URL("../src-tauri/src/sidebar_delete.js", import.meta.url), "utf8");
const context = vm.createContext({});
vm.runInContext(source, context);
const ui = { jsx: (type, props) => ({ type, props }), jsxs: (type, props) => ({ type, props }) };

test("shortcut opens the official confirmation and does not directly delete", () => {
  let confirmed = false;
  const action = context.jokerdeckDeleteAction({ onSelect: () => { confirmed = true; } }, ui);
  assert.equal(action.ariaLabel, "删除会话（需确认）");
  assert.equal(action.icon.type, "svg");
  assert.equal(confirmed, false);
  action.onClick();
  assert.equal(confirmed, true);
});

test("unavailable and disabled native actions never get a shortcut", () => {
  assert.equal(context.jokerdeckDeleteAction(undefined, ui), null);
  assert.equal(context.jokerdeckDeleteAction({ enabled: false, onSelect() {} }, ui), null);
  assert.equal(context.jokerdeckDeleteAction({}, ui), null);
});

test("each shortcut retains its corresponding native thread and host closure", () => {
  const requests = [];
  const first = context.jokerdeckDeleteAction({ onSelect: () => requests.push("host-a/thread-a") }, ui);
  const second = context.jokerdeckDeleteAction({ onSelect: () => requests.push("host-b/thread-b") }, ui);
  second.onClick();
  first.onClick();
  assert.deepEqual(requests, ["host-b/thread-b", "host-a/thread-a"]);
});
