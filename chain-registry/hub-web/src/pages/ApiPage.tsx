import apiMd from "@hub-content/api.md?raw";
import { EXTERNAL_LINKS } from "../config/links";
import { MarkdownContent } from "../components/MarkdownContent";

export function ApiPage() {
  return (
    <>
      <MarkdownContent source={apiMd} />
      <p style={styles.ctaRow}>
        <a href={EXTERNAL_LINKS.apiDocs} style={styles.cta}>
          Open Swagger UI
        </a>
      </p>
    </>
  );
}

const styles = {
  ctaRow: { marginTop: "1.5rem" },
  cta: {
    display: "inline-flex",
    alignItems: "center",
    padding: "0.5rem 1rem",
    borderRadius: "var(--radius-md)",
    background: "var(--accent-primary)",
    color: "#fff",
    fontWeight: 600,
    fontSize: "0.9rem",
    textDecoration: "none",
  },
};
