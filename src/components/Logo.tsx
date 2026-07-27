import markUrl from "../assets/niko-mark.png";

/// 品牌标记：界面内统一用渐变版 mark（应用图标用 src-tauri/icons，托盘用单色版）
export default function Logo({ size = 32 }: { size?: number }) {
  return (
    <img
      src={markUrl}
      width={size}
      height={size}
      alt="Niko"
      className="select-none"
      draggable={false}
    />
  );
}
