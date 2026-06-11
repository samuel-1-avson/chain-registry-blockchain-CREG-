import { NavLink, Outlet } from "react-router-dom";
import { EXTERNAL_LINKS } from "../config/links";
import { HealthBanner } from "./HealthBanner";
import { NetworkGuard } from "./NetworkGuard";
import { WalletButton } from "./WalletButton";

type NavItem = { to: string; label: string; end?: boolean };

const NAV: NavItem[] = [
  { to: "/", label: "Home", end: true },
  { to: "/publish", label: "Publish" },
  { to: "/validate", label: "Validate" },
  { to: "/compare", label: "Compare" },
  { to: "/docs", label: "Docs" },
  { to: "/api-reference", label: "API" },
  { to: "/faq", label: "FAQ" },
];

const EXTERNAL = [
  { href: EXTERNAL_LINKS.explorer, label: "Explorer" },
  { href: EXTERNAL_LINKS.faucet, label: "Faucet" },
] as const;

export function Layout() {
  return (
    <div style={styles.shell}>
      <header style={styles.header}>
        <div style={styles.headerRow}>
          <div style={styles.brandBlock}>
            <NavLink to="/" style={styles.brand}>
              <span style={styles.logo}>C</span>
              <span>
                <span style={styles.brandTitle}>CREG Testnet</span>
                <span style={styles.brandSub}>Join portal</span>
              </span>
            </NavLink>
          </div>
          <nav aria-label="External tools" style={styles.externalNav}>
            {EXTERNAL.map((item) => (
              <a
                key={item.href}
                href={item.href}
                target="_blank"
                rel="noreferrer"
                style={styles.externalLink}
              >
                {item.label}
              </a>
            ))}
          </nav>
          <WalletButton />
        </div>
        <nav aria-label="Primary" style={styles.primaryNav}>
          {NAV.map((item) => (
            <NavLink
              key={item.to}
              to={item.to}
              end={item.end}
              style={({ isActive }) => ({
                ...styles.navLink,
                ...(isActive ? styles.navLinkActive : {}),
              })}
            >
              {item.label}
            </NavLink>
          ))}
        </nav>
      </header>
      <main style={styles.main}>
        <NetworkGuard />
        <HealthBanner />
        <Outlet />
      </main>
      <footer style={styles.footer}>
        <span>CREG Sepolia testnet · chain id 11155111</span>
        <NavLink to="/api-reference" style={{ color: "inherit" }}>
          API reference
        </NavLink>
        <a href={EXTERNAL_LINKS.apiDocs} target="_blank" rel="noreferrer">
          Swagger UI
        </a>
      </footer>
    </div>
  );
}

const styles = {
  shell: {
    minHeight: "100vh",
    display: "flex",
    flexDirection: "column" as const,
  },
  header: {
    position: "sticky" as const,
    top: 0,
    zIndex: "var(--z-sticky)",
    background: "var(--bg-elevated)",
    borderBottom: "1px solid var(--border)",
    backdropFilter: "blur(8px)",
  },
  headerRow: {
    maxWidth: "56rem",
    margin: "0 auto",
    padding: "0.85rem 1.25rem",
    display: "flex",
    alignItems: "center",
    gap: "var(--space-4)",
    flexWrap: "wrap" as const,
  },
  brandBlock: {
    flex: "1 1 auto",
  },
  brand: {
    display: "inline-flex",
    alignItems: "center",
    gap: "0.65rem",
    textDecoration: "none",
    color: "inherit",
  },
  logo: {
    width: 28,
    height: 28,
    borderRadius: "var(--radius-sm)",
    background: "linear-gradient(135deg, var(--accent-primary), #3b82f6)",
    display: "inline-flex",
    alignItems: "center",
    justifyContent: "center",
    color: "#fff",
    fontWeight: 700,
    fontSize: 13,
  },
  brandTitle: {
    display: "block",
    fontWeight: 700,
    fontSize: "0.95rem",
    lineHeight: 1.2,
  },
  brandSub: {
    display: "block",
    fontSize: "0.75rem",
    color: "var(--text-tertiary)",
  },
  externalNav: {
    display: "flex",
    gap: "0.65rem",
    flexWrap: "wrap" as const,
  },
  externalLink: {
    fontSize: "0.85rem",
    color: "var(--text-secondary)",
    textDecoration: "none",
  },
  primaryNav: {
    maxWidth: "56rem",
    margin: "0 auto",
    padding: "0 1.25rem 0.65rem",
    display: "flex",
    gap: "0.35rem",
    flexWrap: "wrap" as const,
  },
  navLink: {
    padding: "0.35rem 0.65rem",
    borderRadius: "var(--radius-sm)",
    color: "var(--text-secondary)",
    textDecoration: "none",
    fontSize: "0.9rem",
    fontWeight: 600,
  },
  navLinkActive: {
    color: "var(--accent-primary-light)",
    background: "rgba(99, 102, 241, 0.12)",
  },
  main: {
    flex: 1,
    maxWidth: "56rem",
    width: "100%",
    margin: "0 auto",
    padding: "1.5rem 1.25rem 3rem",
  },
  footer: {
    borderTop: "1px solid var(--border)",
    padding: "1rem 1.25rem",
    display: "flex",
    justifyContent: "space-between",
    gap: "var(--space-4)",
    flexWrap: "wrap" as const,
    fontSize: "0.85rem",
    color: "var(--text-tertiary)",
  },
};
