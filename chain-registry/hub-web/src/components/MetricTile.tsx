type MetricTileProps = {
  label: string;
  value: string | number | null | undefined;
  hint?: string;
  loading?: boolean;
};

export function MetricTile({ label, value, hint, loading }: MetricTileProps) {
  const showSkeleton = loading && value == null;

  return (
    <article className={`hub-metric${showSkeleton ? " hub-metric-skeleton" : ""}`}>
      <span className="hub-metric-label">{label}</span>
      <strong className="hub-metric-value">{value ?? "--"}</strong>
      {hint && <span className="hub-metric-hint">{hint}</span>}
    </article>
  );
}
