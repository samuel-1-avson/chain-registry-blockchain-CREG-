import { Link } from "react-router-dom";

type PathCardProps = {
  title: string;
  description: string;
  to: string;
  cta: string;
};

export function PathCard({ title, description, to, cta }: PathCardProps) {
  return (
    <article style={styles.card}>
      <h2 style={styles.title}>{title}</h2>
      <p style={styles.body}>{description}</p>
      <Link to={to} style={styles.link}>
        {cta} →
      </Link>
    </article>
  );
}

const styles = {
  card: {
    border: "1px solid var(--border)",
    borderRadius: "var(--radius-lg)",
    padding: "1.25rem",
    background: "var(--surface)",
    display: "flex",
    flexDirection: "column" as const,
    gap: "0.75rem",
    minHeight: "100%",
  },
  title: {
    margin: 0,
    fontSize: "1.2rem",
  },
  body: {
    margin: 0,
    color: "var(--text-secondary)",
    flex: 1,
  },
  link: {
    fontWeight: 600,
    textDecoration: "none",
  },
};
