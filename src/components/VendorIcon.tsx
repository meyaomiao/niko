// 厂商品牌图标：单色小瓷砖，颜色取各家品牌主色
// OpenAI 绿 / Anthropic 陶土色 / Google 四色 G / xAI 黑底 𝕏

const SIZE = "h-[18px] w-[18px] shrink-0 rounded-[5px]";

export function VendorIcon({ vendor }: { vendor: string }) {
  switch (vendor) {
    case "OpenAI":
      return (
        <span
          aria-hidden="true"
          className={`${SIZE} flex items-center justify-center bg-[#10A37F] text-[10px] font-bold text-white`}
        >
          O
        </span>
      );
    case "Anthropic":
      return (
        <span
          aria-hidden="true"
          className={`${SIZE} flex items-center justify-center bg-[#CC785C] text-[10px] font-bold text-white`}
        >
          A
        </span>
      );
    case "Google":
      return (
        <span aria-hidden="true" className={`${SIZE} flex items-center justify-center bg-white`}>
          {/* Google 四色 G */}
          <svg viewBox="0 0 18 18" className="h-[13px] w-[13px]">
            <path
              fill="#4285F4"
              d="M17.64 9.2c0-.64-.06-1.25-.16-1.84H9v3.48h4.84a4.14 4.14 0 0 1-1.8 2.72v2.26h2.92c1.7-1.57 2.68-3.88 2.68-6.62Z"
            />
            <path
              fill="#34A853"
              d="M9 18c2.43 0 4.47-.8 5.96-2.18l-2.92-2.26c-.8.54-1.84.86-3.04.86-2.34 0-4.32-1.58-5.03-3.7H.96v2.32A9 9 0 0 0 9 18Z"
            />
            <path
              fill="#FBBC05"
              d="M3.97 10.72a5.41 5.41 0 0 1 0-3.44V4.96H.96a9 9 0 0 0 0 8.08l3.01-2.32Z"
            />
            <path
              fill="#EA4335"
              d="M9 3.58c1.32 0 2.5.45 3.44 1.35l2.58-2.58C13.46.9 11.42 0 9 0A9 9 0 0 0 .96 4.96l3.01 2.32C4.68 5.16 6.66 3.58 9 3.58Z"
            />
          </svg>
        </span>
      );
    case "xAI":
      return (
        <span
          aria-hidden="true"
          className={`${SIZE} flex items-center justify-center bg-gray-900 text-[10px] font-bold text-white dark:bg-white dark:text-gray-900`}
        >
          𝕏
        </span>
      );
    default:
      return (
        <span
          aria-hidden="true"
          className={`${SIZE} flex items-center justify-center bg-black/5 text-[10px] font-semibold text-gray-500 dark:bg-white/10 dark:text-gray-400`}
        >
          ·
        </span>
      );
  }
}
