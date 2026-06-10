import validateMd from "@hub-content/validate.md?raw";
import { MarkdownContent } from "../components/MarkdownContent";

export function ValidatePage() {
  return <MarkdownContent source={validateMd} />;
}
