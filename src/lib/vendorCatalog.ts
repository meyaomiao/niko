// 厂家目录：把服务端下发的 model → vendor_id 与公开 vendors 表连接起来
// 服务端新版 bootstrap 的 pricing[] 带 vendor_id，公开 /api/pricing 提供
// vendors[]（id/name/icon）。两者都有才能拿到权威厂家归属；
// 任一缺失时调用方回退到 src/lib/vendor.ts 的名称启发式。

import type { PricingItem, VendorMeta } from "../api/client.ts";
import { vendorOfModel } from "./vendor.ts";

export interface VendorInfo {
  name: string;
  icon?: string;
}

/** 模型名 → 厂家信息（仅服务端有归属的模型） */
export function buildVendorIndex(
  pricing: PricingItem[] | undefined,
  vendors: VendorMeta[] | undefined
): Map<string, VendorInfo> {
  const index = new Map<string, VendorInfo>();
  if (!pricing?.length || !vendors?.length) return index;

  const byId = new Map<number, VendorMeta>();
  for (const vendor of vendors) byId.set(vendor.id, vendor);

  for (const item of pricing) {
    if (!item.model_name || item.vendor_id === undefined) continue;
    const vendor = byId.get(item.vendor_id);
    if (!vendor?.name) continue;
    index.set(item.model_name, { name: vendor.name, icon: vendor.icon });
  }
  return index;
}

/** 取模型的厂家名：服务端优先，没有则用本地启发式 */
export function vendorNameOf(index: Map<string, VendorInfo>, model: string): string {
  return index.get(model)?.name ?? vendorOfModel(model);
}

/** 该模型的官方端点能力（服务端下发），用于识别生图/生视频模型 */
export function endpointTypesOf(
  pricing: PricingItem[] | undefined,
  model: string
): string[] {
  return pricing?.find((item) => item.model_name === model)?.supported_endpoint_types ?? [];
}
