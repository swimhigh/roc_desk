import React from "react";
import { useConnectionStore } from "../../stores/connectionStore";
import { HostKeyDialog } from "./HostKeyDialog";

/**
 * 挂载在 App 顶层的全局宿主：把 `ssh:host-key-prompt` 事件渲染成
 * TOFU / 指纹变化弹窗（DESIGN.md §3.2.1），任何工作区/终端触发的握手都走这一个实例，
 * 不需要每个终端标签各自维护一份。
 */
export const HostKeyPromptHost: React.FC = () => {
  const prompt = useConnectionStore((s) => s.pendingHostKeyPrompt);
  const resolve = useConnectionStore((s) => s.resolveHostKeyPrompt);

  if (!prompt) return null;

  if (!prompt.changed) {
    return (
      <HostKeyDialog
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
    <HostKeyDialog
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
