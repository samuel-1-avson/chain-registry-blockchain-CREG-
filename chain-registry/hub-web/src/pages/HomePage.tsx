import homeMd from "@hub-content/home.md?raw";
import { PathCard } from "../components/PathCard";
import { MarkdownContent } from "../components/MarkdownContent";

export function HomePage() {
  return (
    <>
      <MarkdownContent source={homeMd} />
      <section style={styles.cards} aria-label="Contribution paths">
        <PathCard
          title="Publish packages"
          description="Ship signed packages to the Sepolia lab and verify them on the explorer."
          to="/publish"
          cta="Publish guide"
        />
        <PathCard
          title="Run a validator"
          description="Stake, register, and operate a node with the operator runbook."
          to="/validate"
          cta="Validate guide"
        />
      </section>
    </>
  );
}

const styles = {
  cards: {
    display: "grid",
    gap: "1rem",
    gridTemplateColumns: "repeat(auto-fit, minmax(16rem, 1fr))",
    marginTop: "var(--space-6)",
  },
};
