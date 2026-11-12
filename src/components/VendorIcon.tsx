import { resolveVendorIcon } from "../lib/vendorIcons";

const SIZE = "h-[18px] w-[18px] shrink-0 rounded-[5px]";

export function VendorIcon({ vendor, icon, size }: { vendor: string; icon?: string | null; size?: number }) {
  const dimension = size && Number.isFinite(size) && size > 0 ? { width: size, height: size } : undefined;
  const asset = resolveVendorIcon(vendor, icon);

  return (
    <span
      aria-hidden="true"
      data-vendor-icon={asset?.id ?? "fallback"}
      className={`${SIZE} inline-flex items-center justify-center ${asset ? "" : "bg-black/[0.05] text-gray-400 dark:bg-white/10 dark:text-gray-500"}`}
      style={dimension}
      title={asset ? undefined : `${vendor.trim() || "此厂商"}：暂无可核实品牌图标`}
    >
      {asset ? (
        <img
          src={asset.src}
          alt=""
          draggable={false}
          className={`h-full w-full object-contain ${asset.monochrome ? "dark:invert" : ""}`}
        />
      ) : (
        <svg viewBox="0 0 24 24" className="h-3/4 w-3/4" fill="none" stroke="currentColor" strokeWidth="1.5">
          <rect x="3" y="3" width="18" height="18" rx="4" />
          <path d="M9 9a3 3 0 0 1 6 0c0 2-3 2-3 4" strokeLinecap="round" />
          <circle cx="12" cy="17" r=".8" fill="currentColor" stroke="none" />
        </svg>
      )}
    </span>
  );
}
