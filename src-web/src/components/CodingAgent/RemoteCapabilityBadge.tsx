import React from "react";

interface RemoteCapabilityBadgeProps {
  onLearnMore?: () => void;
}

/** "远程模式：无智能补全" 常驻徽标（DESIGN.md §3.8.7）。本地工作区不渲染这个组件。*/
export const RemoteCapabilityBadge: React.FC<RemoteCapabilityBadgeProps> = ({ onLearnMore }) => (
  <span
    className="remote-capability-badge"
    title="远程工作区暂不支持语言服务器级智能感知（跳转定义/实时诊断/语义补全），编辑器仅提供基础语法高亮。"
    onClick={onLearnMore}
  >
    远程模式：无智能补全
  </span>
);
