// 品牌标记：与应用图标同形（圆角渐变底 + 已启用状态的开关）
export default function Logo({ size = 32 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 64 64" role="img" aria-label="Piko">
      <defs>
        <linearGradient id="piko-bg" x1="0" y1="0" x2="0" y2="1">
          <stop offset="0%" stopColor="#6366F1" />
          <stop offset="100%" stopColor="#8B5CF6" />
        </linearGradient>
      </defs>
      <rect width="64" height="64" rx="15" fill="url(#piko-bg)" />
      <rect x="12.8" y="21.1" width="38.4" height="21.8" rx="10.9" fill="#fff" />
      <circle cx="41.6" cy="32" r="7.4" fill="url(#piko-bg)" />
    </svg>
  );
}
