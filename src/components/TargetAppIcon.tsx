import chatgptIcon from "../assets/target-apps/chatgpt.png";
import claudeIcon from "../assets/target-apps/claude.png";

interface TargetAppIconProps {
  targetId: string;
  name: string;
  icon?: string | null;
  size?: "sm" | "md" | "lg";
}

const TARGET_FALLBACKS: Record<string, { icon?: string; label: string; className: string }> = {
  codex: {
    icon: chatgptIcon,
    label: "G",
    className: "bg-[var(--nk-info-soft)] text-[var(--nk-info)]",
  },
  "claude-desktop": {
    icon: claudeIcon,
    label: "C",
    className: "bg-[var(--nk-warning-soft)] text-[var(--nk-accent)]",
  },
};

export default function TargetAppIcon({
  targetId,
  name,
  icon,
  size = "sm",
}: TargetAppIconProps) {
  const fallback = TARGET_FALLBACKS[targetId] ?? {
    label: "·",
    className: "bg-[var(--nk-surface-muted)] text-gray-500",
  };
  const sizeClass =
    size === "lg"
      ? "h-16 w-16 rounded-2xl text-base"
      : size === "md"
        ? "h-10 w-10 rounded-xl text-sm"
        : "h-7 w-7 rounded-lg text-xs";
  const resolvedIcon = icon || fallback.icon;

  return (
    <span
      role="img"
      aria-label={`${name} 图标`}
      className={`relative flex shrink-0 items-center justify-center overflow-hidden border font-semibold shadow-sm [border-color:var(--nk-line)] ${sizeClass} ${fallback.className}`}
    >
      <span aria-hidden="true">{fallback.label}</span>
      {resolvedIcon && (
        <img
          src={resolvedIcon}
          alt=""
          className="absolute inset-0 h-full w-full object-contain"
          onError={(event) => {
            event.currentTarget.style.display = "none";
          }}
        />
      )}
    </span>
  );
}
