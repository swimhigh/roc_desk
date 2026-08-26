import React, { useState } from "react";
import { ConfirmDialog } from "../shared/ConfirmDialog";

interface QuestionDialogProps {
  open: boolean;
  question: string;
  options: string[];
  onAnswer: (answer: string) => void;
}

/**
 * `question` 工具的结构化提问弹窗（REQUIREMENTS.md §3.7）：模型在任务中间需要
 * 用户澄清/决策时调用，和把问题混在最终答案文字里不同——这里会真的阻塞对话
 * 继续，直到用户在这个弹窗里给出回答。有 `options` 时渲染按钮组（点了立即
 * 提交），否则渲染一个文本输入框 + 提交按钮。不提供"拒绝回答"——用户要么答，
 * 要么就不回这条消息（弹窗没有取消按钮，和 CommandConfirmDialog 的可拒绝语义
 * 不同：拒绝执行命令是一个明确的选项，"拒绝回答问题"不是模型能理解的工具结果）。
 */
export const QuestionDialog: React.FC<QuestionDialogProps> = ({ open, question, options, onAnswer }) => {
  const [text, setText] = useState("");

  React.useEffect(() => {
    if (open) setText("");
  }, [open, question]);

  return (
    <ConfirmDialog
      open={open}
      severity="info"
      icon="❓"
      title="AI 需要你澄清一下"
      dismissible={false}
      actions={
        options.length === 0 ? (
          <button className="btn primary sm" onClick={() => onAnswer(text)} disabled={!text.trim()}>
            提交回答
          </button>
        ) : null
      }
    >
      <p style={{ fontSize: 13, marginBottom: 10 }}>{question}</p>
      {options.length > 0 ? (
        <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
          {options.map((option) => (
            <button key={option} className="btn ghost sm" style={{ justifyContent: "flex-start" }} onClick={() => onAnswer(option)}>
              {option}
            </button>
          ))}
        </div>
      ) : (
        <textarea
          className="form-input"
          rows={3}
          autoFocus
          value={text}
          onChange={(e) => setText(e.target.value)}
          placeholder="输入你的回答…"
        />
      )}
    </ConfirmDialog>
  );
};
