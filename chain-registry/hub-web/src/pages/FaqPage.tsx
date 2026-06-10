import faqMd from "@hub-content/faq.md?raw";
import { MarkdownContent } from "../components/MarkdownContent";

export function FaqPage() {
  return <MarkdownContent source={faqMd} />;
}
