export type DetectionStatus = "checking" | "success" | "error";

export interface TargetInfo {
  id: string;
  name: string;
  installed: boolean;
  icon?: string | null;
}

export interface LoginTarget extends TargetInfo {
  downloadUrl: string;
}

interface TargetDefinition {
  id: string;
  name: string;
  downloadUrl: string;
}

export const LOGIN_TARGETS: readonly TargetDefinition[] = [
  {
    id: "codex",
    name: "ChatGPT 桌面端",
    downloadUrl: "https://chatgpt.com/download/",
  },
  {
    id: "claude-desktop",
    name: "Claude 桌面端",
    downloadUrl: "https://claude.com/download",
  },
];

export type TargetRenderState = "checking" | "error" | "installed" | "missing";

/** 后端顺序变化或暂缺某项时，登录页仍稳定展示固定的两个接入目标。 */
export function mapLoginTargets(targets: TargetInfo[]): LoginTarget[] {
  const byId = new Map(targets.map((target) => [target.id, target]));

  return LOGIN_TARGETS.map((definition) => {
    const target = byId.get(definition.id);
    return {
      ...definition,
      name: target?.name?.trim() || definition.name,
      installed: target?.installed ?? false,
      icon: target?.icon || null,
    };
  });
}

export function getTargetRenderState(
  detectionStatus: DetectionStatus,
  installed: boolean
): TargetRenderState {
  if (detectionStatus === "checking") return "checking";
  if (detectionStatus === "error") return "error";
  return installed ? "installed" : "missing";
}
