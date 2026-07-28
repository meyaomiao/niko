import assert from "node:assert/strict";
import test from "node:test";
import {
  getTargetRenderState,
  LOGIN_TARGETS,
  mapLoginTargets,
} from "../src/lib/loginTargets.ts";

test("maps backend results into the fixed login-page order", () => {
  const mapped = mapLoginTargets([
    {
      id: "claude-desktop",
      name: "Claude 桌面端",
      installed: true,
      icon: "data:image/png;base64,claude",
    },
    {
      id: "codex",
      name: "ChatGPT 桌面端",
      installed: false,
      icon: null,
    },
  ]);

  assert.deepEqual(mapped.map((target) => target.id), ["codex", "claude-desktop"]);
  assert.equal(mapped[0].installed, false);
  assert.equal(mapped[1].installed, true);
  assert.equal(mapped[1].icon, "data:image/png;base64,claude");
});

test("fills an omitted target with stable metadata and the official download URL", () => {
  const mapped = mapLoginTargets([]);

  assert.equal(mapped.length, 2);
  assert.deepEqual(
    mapped.map(({ name, installed, icon }) => ({ name, installed, icon })),
    [
      { name: "ChatGPT 桌面端", installed: false, icon: null },
      { name: "Claude 桌面端", installed: false, icon: null },
    ]
  );
  assert.deepEqual(
    LOGIN_TARGETS.map((target) => target.downloadUrl),
    ["https://chatgpt.com/download/", "https://claude.com/download"]
  );
});

test("selects the key render branch from detection and installation state", () => {
  assert.equal(getTargetRenderState("checking", false), "checking");
  assert.equal(getTargetRenderState("error", true), "error");
  assert.equal(getTargetRenderState("success", true), "installed");
  assert.equal(getTargetRenderState("success", false), "missing");
});
