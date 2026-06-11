import { useState } from "react";

type CommandBlockProps = {
  label: string;
  command: string;
};

export function CommandBlock({ label, command }: CommandBlockProps) {
  const [copied, setCopied] = useState(false);
  const [copying, setCopying] = useState(false);

  async function copy() {
    if (copying) return;
    setCopying(true);
    try {
      await navigator.clipboard.writeText(command);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1400);
    } finally {
      setCopying(false);
    }
  }

  return (
    <div className="hub-command">
      <div className="hub-command-header">
        <span>{label}</span>
        <button type="button" disabled={copying} onClick={() => void copy()}>
          {copied ? "Copied" : copying ? "…" : "Copy"}
        </button>
      </div>
      <pre>{command}</pre>
    </div>
  );
}
