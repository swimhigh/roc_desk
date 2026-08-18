import React from "react";
import { Sun, Moon } from "lucide-react";
import { useThemeStore } from "../../stores/themeStore";

export const ThemeToggle: React.FC<{ className?: string }> = ({ className }) => {
  const { theme, toggle } = useThemeStore();
  return (
    <button
      className={className ?? "quick-tool-btn"}
      onClick={toggle}
      title={theme === "dark" ? "切换到浅色主题" : "切换到深色主题"}
    >
      {theme === "dark" ? <Sun /> : <Moon />}
    </button>
  );
};
