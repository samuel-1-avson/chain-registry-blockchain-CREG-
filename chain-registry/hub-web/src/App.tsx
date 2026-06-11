import { BrowserRouter, Navigate, Route, Routes } from "react-router-dom";
import { Layout } from "./components/Layout";
import { ApiPage } from "./pages/ApiPage";
import { ComparePage } from "./pages/ComparePage";
import { DashboardPage } from "./pages/DashboardPage";
import { DocsPage } from "./pages/DocsPage";
import { FaqPage } from "./pages/FaqPage";
import { HomePage } from "./pages/HomePage";
import { NetworkPage } from "./pages/NetworkPage";
import { ObserverPage } from "./pages/ObserverPage";
import { PublishPage } from "./pages/PublishPage";
import { ValidatePage } from "./pages/ValidatePage";

export function App() {
  return (
    <BrowserRouter>
      <Routes>
        <Route element={<Layout />}>
          <Route index element={<HomePage />} />
          <Route path="observer" element={<ObserverPage />} />
          <Route path="publish" element={<PublishPage />} />
          <Route path="validate" element={<ValidatePage />} />
          <Route path="network" element={<NetworkPage />} />
          <Route path="dashboard" element={<DashboardPage />} />
          <Route path="compare" element={<ComparePage />} />
          <Route path="faq" element={<FaqPage />} />
          <Route path="docs" element={<DocsPage />} />
          <Route path="api-reference" element={<ApiPage />} />
          <Route path="*" element={<Navigate to="/" replace />} />
        </Route>
      </Routes>
    </BrowserRouter>
  );
}
