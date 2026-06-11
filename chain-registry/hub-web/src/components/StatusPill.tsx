type StatusPillProps = {
  tone?: "success" | "warning" | "error" | "info" | "muted";
  children: string;
};

export function StatusPill({ tone = "muted", children }: StatusPillProps) {
  return <span className={`hub-pill hub-pill-${tone}`}>{children}</span>;
}
