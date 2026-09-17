import assert from "node:assert/strict";
import test from "node:test";
import {
  assembleNewApiBootstrap,
  assembleNewApiPricingMeta,
  mapNewApiLogs,
  summarizeNewApiLogs,
} from "../src/lib/newapi.ts";

test("assembles bootstrap from new-api status, groups, models and pricing", () => {
  const bootstrap = assembleNewApiBootstrap({
    origin: "https://relay.example.com",
    status: {
      success: true,
      data: { system_name: "DeepKey", version: "0.9", quota_per_unit: 500000 },
    },
    user: { success: true, data: { id: 10085, username: "xia", quota: 123, group: "claude" } },
    groups: {
      success: true,
      data: {
        claude: { desc: "Claude 分组", ratio: 0.4 },
        default: { desc: "默认", ratio: 1 },
      },
    },
    models: { success: true, data: ["claude-sonnet-4-6", "gpt-5"] },
    pricing: {
      success: true,
      data: [
        {
          model_name: "claude-sonnet-4-6",
          quota_type: 0,
          model_ratio: 3,
          model_price: 0,
          completion_ratio: 5,
          enable_groups: ["claude"],
          vendor_id: 2,
        },
      ],
      vendors: [{ id: 2, name: "Anthropic", icon: "Claude.Color" }],
      usable_group: { claude: "Claude 分组", default: "默认" },
      group_ratio: { claude: 0.4, default: 1 },
    },
  });

  assert.equal(bootstrap.site.base_url, "https://relay.example.com");
  assert.equal(bootstrap.site.system_name, "DeepKey");
  assert.equal(bootstrap.site.quota_per_unit, 500000);
  assert.equal(bootstrap.user.id, 10085);
  assert.equal(bootstrap.user.group, "claude");
  assert.deepEqual(bootstrap.groups?.map((group) => group.name).sort(), ["claude", "default"]);
  assert.equal(bootstrap.user.group, "claude");
  assert.equal(bootstrap.pricing[0]?.enable_groups?.[0], "claude");
  assert.deepEqual(bootstrap.models, ["claude-sonnet-4-6", "gpt-5"]);
});

test("splits comma-separated user groups into account groups", () => {
  const bootstrap = assembleNewApiBootstrap({
    origin: "https://relay.example.com",
    status: { quota_per_unit: 500000 },
    user: { id: 1, quota: 10, group: "claude,default" },
    groups: null,
    models: ["gpt-5"],
    pricing: { data: [] },
  });
  assert.deepEqual(bootstrap.groups?.map((group) => group.name), ["claude", "default"]);
  assert.equal(bootstrap.user.group, "claude");
});

test("reads vendor catalog from the public pricing envelope", () => {
  const meta = assembleNewApiPricingMeta({
    vendors: [{ id: 1, name: "OpenAI" }],
    usable_group: { default: "默认分组" },
    group_ratio: { default: 1.5 },
    data: [],
  });
  assert.deepEqual(meta.vendors, [{ id: 1, name: "OpenAI", description: undefined, icon: undefined }]);
  assert.equal(meta.usableGroup.default, "默认分组");
  assert.equal(meta.groupRatio.default, 1.5);
});

test("maps and summarizes new-api usage logs", () => {
  const mapped = mapNewApiLogs({
    success: true,
    data: {
      total: 2,
      items: [
        {
          id: 1,
          created_at: 1_700_000_000,
          model_name: "gpt-5",
          prompt_tokens: 10,
          completion_tokens: 20,
          quota: 100,
          group: "default",
          is_stream: true,
        },
        {
          id: 2,
          created_at: 1_700_000_100,
          model_name: "gpt-5",
          prompt_tokens: 5,
          completion_tokens: 5,
          quota: 50,
          group: "default",
          is_stream: false,
        },
      ],
    },
  });
  assert.equal(mapped.total, 2);
  const summary = summarizeNewApiLogs(mapped.items);
  assert.equal(summary.quota, 150);
  assert.equal(summary.requests, 2);
  assert.equal(summary.stream_requests, 1);
  assert.equal(summary.by_model?.[0]?.name, "gpt-5");
  assert.equal(summary.by_group?.[0]?.name, "default");
});
