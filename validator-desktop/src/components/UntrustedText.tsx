import { useState } from "react";

const CONTROL_AND_BIDI_RE =
  /[\u0000-\u0008\u000B\u000C\u000E-\u001F\u007F-\u009F\u200E\u200F\u061C\u202A-\u202E\u2066-\u2069]/g;

function sanitize(raw: unknown): string {
  if (raw == null) return "";
  return String(raw).replace(/\r\n?/g, "\n").replace(CONTROL_AND_BIDI_RE, "");
}

type SegmenterCtorLike = new (
  locale?: string,
  options?: { granularity: "grapheme" },
) => { segment: (input: string) => Iterable<{ segment: string }> };

function truncateGraphemes(text: string, maxGraphemes: number) {
  const SegmenterCtor = (Intl as unknown as { Segmenter?: SegmenterCtorLike }).Segmenter;
  const units = SegmenterCtor
    ? Array.from(
        new SegmenterCtor(undefined, { granularity: "grapheme" }).segment(text),
        (s) => s.segment,
      )
    : Array.from(text);
  if (units.length <= maxGraphemes) {
    return { text, truncated: false };
  }
  return { text: `${units.slice(0, maxGraphemes).join("")}...`, truncated: true };
}

export function UntrustedText({
  value,
  maxGraphemes = 128,
  expandable = false,
  label = "untrusted",
}: {
  value: unknown;
  maxGraphemes?: number;
  expandable?: boolean;
  label?: string;
}) {
  const [expanded, setExpanded] = useState(false);
  const clean = sanitize(value);
  const { text, truncated } = truncateGraphemes(clean, maxGraphemes);
  const display = expanded ? clean : text;

  return (
    <span className="untrusted-wrap">
      <span
        data-untrusted="true"
        className="untrusted-value"
        title={truncated ? `Untrusted (capped at ${maxGraphemes} graphemes)` : "Untrusted text"}
      >
        {display}
      </span>
      <span className="untrusted-tag">{label}</span>
      {expandable && truncated && (
        <button
          type="button"
          className="link-btn"
          onClick={() => setExpanded((v) => !v)}
        >
          {expanded ? "Show less" : "Show full"}
        </button>
      )}
    </span>
  );
}
