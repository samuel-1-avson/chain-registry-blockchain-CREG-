import compareMd from "@hub-content/compare.md?raw";
import { MarkdownContent } from "../components/MarkdownContent";

export function ComparePage() {
  return <MarkdownContent source={compareMd} />;
}
