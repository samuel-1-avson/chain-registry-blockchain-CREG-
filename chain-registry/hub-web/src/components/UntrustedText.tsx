/**
 * Render attacker-influenced strings safely as plain text with bidi isolation
 * and grapheme truncation (ADR-019 ECO-INV-05).
 */
const CONTROL_AND_BIDI_RE =
  /[\u0000-\u0008\u000B\u000C\u000E-\u001F\u007F-\u009F\u200E\u200F\u061C\u202A-\u202E\u2066-\u2069]/g;

function sanitizeText(raw: unknown): string {
  if (raw == null) return "";
  return String(raw).replace(/\r\n?/g, "\n").replace(CONTROL_AND_BIDI_RE, "");
}

function truncateGraphemes(text: string, maxGraphemes: number): { text: string; truncated: boolean } {
  const SegmenterCtor = (Intl as { Segmenter?: typeof Intl.Segmenter }).Segmenter;
  const units = SegmenterCtor
    ? Array.from(new SegmenterCtor(undefined, { granularity: "grapheme" }).segment(text), (s) => s.segment)
    : Array.from(text);
  if (units.length <= maxGraphemes) {
    return { text, truncated: false };
  }
  return {
    text: `${units.slice(0, maxGraphemes).join("")}...`,
    truncated: true,
  };
}

export function UntrustedText({
  value,
  maxGraphemes = 128,
}: {
  value: unknown;
  maxGraphemes?: number;
}) {
  const sanitized = sanitizeText(value);
  const { text, truncated } = truncateGraphemes(sanitized, maxGraphemes);
  return (
    <span
      data-untrusted="true"
      title={truncated ? `Untrusted value (truncated to ${maxGraphemes} graphemes)` : "Untrusted value"}
      style={{
        unicodeBidi: "isolate",
        whiteSpace: "pre-wrap",
        fontFamily: "var(--font-mono, ui-monospace, SFMono-Regular, Menlo, monospace)",
        wordBreak: "break-word",
      }}
    >
      {text}
      <span
        style={{
          marginLeft: "0.4em",
          fontSize: "0.7em",
          opacity: 0.7,
          border: "1px solid currentColor",
          borderRadius: "4px",
          padding: "0 4px",
          verticalAlign: "middle",
        }}
      >
        untrusted
      </span>
    </span>
  );
}
