import logoUrl from "../assets/niko-logo-horizontal.svg";
import whiteLogoUrl from "../assets/niko-logo-white.svg";

/// 品牌签名：界面内统一使用 C 方案的定制英文组合 Logo。
export default function Logo({ size = 32 }: { size?: number }) {
  const width = Math.round((size * 1280) / 460);

  return (
    <span
      role="img"
      aria-label="Niko"
      className="inline-block shrink-0 select-none"
      style={{ width, height: size }}
    >
      <img
        src={logoUrl}
        width={width}
        height={size}
        alt=""
        className="block h-full w-full dark:hidden"
        draggable={false}
      />
      <img
        src={whiteLogoUrl}
        width={width}
        height={size}
        alt=""
        className="hidden h-full w-full dark:block"
        draggable={false}
      />
    </span>
  );
}
