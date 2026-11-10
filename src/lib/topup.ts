import type { EpayOrder, TopUpRecord } from "../api/client.ts";

/** 易支付的商户订单号对应充值记录的 trade_no。 */
export function epayTradeNo(order: EpayOrder): string | null {
  const value = order.params.out_trade_no;
  if (typeof value !== "string" || !value.trim()) return null;
  return value.trim();
}

/** 余额可能同时被消费扣减或由其它订单增加，只确认当前订单的成功记录。 */
export function isTopupPaid(tradeNo: string | null, records: TopUpRecord[] | null | undefined): boolean {
  return Boolean(tradeNo && records?.some((record) => record.trade_no === tradeNo && record.status === "success"));
}
