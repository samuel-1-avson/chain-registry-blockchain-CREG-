import docsMd from "@hub-content/docs.md?raw";
import { MarkdownContent } from "../components/MarkdownContent";

export function DocsPage() {
  return <MarkdownContent source={docsMd} />;
}
