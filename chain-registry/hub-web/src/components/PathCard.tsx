import { Link } from "react-router-dom";

type PathKind = "observe" | "publish" | "validate" | "docs";

type PathCardProps = {
  title: string;
  description: string;
  to: string;
  cta: string;
  path?: PathKind;
  index?: number;
};

export function PathCard({
  title,
  description,
  to,
  cta,
  path = "observe",
  index,
}: PathCardProps) {
  return (
    <article className="hub-path-card" data-path={path}>
      {index != null && (
        <span className="hub-path-index">
          {String(index).padStart(2, "0")}
        </span>
      )}
      <h2>{title}</h2>
      <p>{description}</p>
      <Link to={to} className="hub-button-secondary">
        {cta}
      </Link>
    </article>
  );
}
