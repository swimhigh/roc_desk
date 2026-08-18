import React from "react";

interface ToggleSwitchProps {
  checked: boolean;
  onChange?: (next: boolean) => void;
  disabled?: boolean;
  label?: string;
}

/** 设置页开关，支持 disabled 态（如"高危命令黑名单拦截"不可关闭，DESIGN.md §3.8.2.1）。*/
export const ToggleSwitch: React.FC<ToggleSwitchProps> = ({ checked, onChange, disabled, label }) => (
  <span
    className={`toggle-switch ${checked ? "on" : ""} ${disabled ? "disabled" : ""}`}
    role="switch"
    aria-checked={checked}
    aria-label={label}
    onClick={() => !disabled && onChange?.(!checked)}
  />
);
