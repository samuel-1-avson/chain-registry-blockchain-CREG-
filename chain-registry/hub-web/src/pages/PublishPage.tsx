import publishMd from "@hub-content/publish.md?raw";
import { MarkdownContent } from "../components/MarkdownContent";

export function PublishPage() {
  return <MarkdownContent source={publishMd} />;
}
