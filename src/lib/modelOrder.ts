// 模型列表统一排序：不管有没有筛选厂家，都用同一套规则。
// 1. 有官方发布日期的，按发布日期从新到旧；
// 2. 没有日期的排在有日期之后，并按服务端发布顺序（model_order）靠前；
//    bootstrap 里 model_order 的顺序本身就是发布先后，所以它是「无日期」的正当代理；
// 3. 两者都缺时按名称，保证排序稳定可复现。

export interface ModelOrderContext {
  /** 发布日期时间戳；没有日期返回 null */
  dateOf: (name: string) => number | null;
  /** 服务端发布顺序下标；不在目录中返回 undefined */
  orderOf: (name: string) => number | undefined;
}

export function compareModelsByRelease(
  a: string,
  b: string,
  ctx: ModelOrderContext
): number {
  const ta = ctx.dateOf(a);
  const tb = ctx.dateOf(b);
  if (ta !== null && tb !== null && ta !== tb) return tb - ta;
  if ((ta !== null) !== (tb !== null)) return ta !== null ? -1 : 1;

  const ia = ctx.orderOf(a);
  const ib = ctx.orderOf(b);
  if (ia !== undefined && ib !== undefined && ia !== ib) return ia - ib;
  if ((ia !== undefined) !== (ib !== undefined)) return ia !== undefined ? -1 : 1;

  return a.localeCompare(b);
}

/** 便捷包装：直接排一个模型名数组（不改原数组） */
export function sortModelNames(names: string[], ctx: ModelOrderContext): string[] {
  return [...names].sort((a, b) => compareModelsByRelease(a, b, ctx));
}
