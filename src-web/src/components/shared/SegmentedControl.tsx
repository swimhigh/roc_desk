import React from "react";

interface SegmentedOption<T extends string> {
  value: T;
  label: React.ReactNode;
}

interface SegmentedControlProps<T extends string> {
  value: T;
  options: SegmentedOption<T>[];
  onChange: (value: T) => void;
}

/** 二选一控件（如日志搜索"实时/索引"），统一收口，不在各模块各画一套。*/
export function SegmentedControl<T extends string>({ value, options, onChange }: SegmentedControlProps<T>) {
  return (
    <div className="segmented">
      {options.map((opt) => (
        <div
          key={opt.value}
          className={`seg-btn ${opt.value === value ? "active" : ""}`}
          onClick={() => onChange(opt.value)}
        >
          {opt.label}
        </div>
      ))}
    </div>
  );
}
