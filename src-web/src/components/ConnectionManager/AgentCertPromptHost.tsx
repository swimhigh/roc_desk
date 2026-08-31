import React from "react";
import { useConnectionStore } from "../../stores/connectionStore";
import { AgentCertDialog } from "./AgentCertDialog";

/**
 * 挂载在 App 顶层的全局宿主：把 `agent:cert-prompt` 事件渲染成 TOFU / 指纹变化
 * 弹窗（AGENT_DESIGN.md §3.1），和 `HostKeyPromptHost` 是同一种模式。
 */
export const AgentCertPromptHost: React.FC = () => {
  const prompt = useConnectionStore((s) => s.pendingAgentCertPrompt);
  const resolve = useConnectionStore((s) => s.resolveAgentCertPrompt);

  if (!prompt) return null;

  if (!prompt.changed) {
    return (
      <AgentCertDialog
        open
        kind="tofu"
        host={prompt.host}
        fingerprint={prompt.fingerprint}
        onCancel={() => resolve(false)}
        onTrust={() => resolve(true)}
      />
    );
  }

  return (
    <AgentCertDialog
      open
      kind="changed"
      host={prompt.host}
      oldFingerprint={prompt.oldFingerprint ?? ""}
      newFingerprint={prompt.fingerprint}
      onCancel={() => resolve(false)}
      onTrustAnyway={() => resolve(true)}
    />
  );
};
