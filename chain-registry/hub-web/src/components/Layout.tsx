import { NavLink, Outlet } from "react-router-dom";
import { EXTERNAL_LINKS } from "../config/links";
import { AlphaDisclaimer } from "./AlphaDisclaimer";
import { HealthBanner } from "./HealthBanner";
import { NetworkGuard } from "./NetworkGuard";
import { WalletButton } from "./WalletButton";

type NavItem = { to: string; label: string; end?: boolean };

const NAV: NavItem[] = [
  { to: "/", label: "Start", end: true },
  { to: "/observer", label: "Observe" },
  { to: "/publish", label: "Publish" },
  { to: "/validate", label: "Validate" },
  { to: "/network", label: "Network" },
  { to: "/dashboard", label: "Dashboard" },
  { to: "/docs", label: "Docs" },
];

const EXTERNAL = [
  { href: EXTERNAL_LINKS.explorer, label: "Explorer" },
  { href: EXTERNAL_LINKS.faucet, label: "Faucet" },
  { href: EXTERNAL_LINKS.waitlist, label: "Waitlist" },
] as const;

export function Layout() {
  return (
    <div className="hub-shell">
      <a href="#main-content" className="hub-skip">
        Skip to content
      </a>
      <header className="hub-header">
        <div className="hub-header-inner">
          <NavLink to="/" className="hub-brand">
            <strong>CREG Testnet</strong>
            <span>Public alpha join hub</span>
          </NavLink>
          <nav aria-label="Primary" className="hub-nav">
            {NAV.map((item) => (
              <NavLink key={item.to} to={item.to} end={item.end}>
                {item.label}
              </NavLink>
            ))}
            {EXTERNAL.map((item) => (
              <a
                key={item.href}
                href={item.href}
                target="_blank"
                rel="noreferrer"
                className="hub-nav-external"
              >
                {item.label}
              </a>
            ))}
          </nav>
          <div className="hub-header-actions">
            <WalletButton />
          </div>
        </div>
      </header>
      <main id="main-content" className="hub-main">
        <NetworkGuard />
        <HealthBanner />
        <AlphaDisclaimer compact />
        <Outlet />
      </main>
      <footer className="hub-footer">
        <div className="hub-footer-inner">
          <p>CREG Sepolia testnet · chain id 11155111</p>
          <nav aria-label="Footer">
            <NavLink to="/compare">Compare paths</NavLink>
            <NavLink to="/faq">FAQ</NavLink>
            <NavLink to="/api-reference">API reference</NavLink>
            <a href={EXTERNAL_LINKS.apiDocs} target="_blank" rel="noreferrer">
              Swagger UI
            </a>
          </nav>
        </div>
      </footer>
    </div>
  );
}
