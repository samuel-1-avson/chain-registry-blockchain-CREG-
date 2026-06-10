import { renderMarkdown } from "../lib/markdown";

type MarkdownContentProps = {
  source: string;
};

export function MarkdownContent({ source }: MarkdownContentProps) {
  return (
    <article
      className="markdown-body"
      dangerouslySetInnerHTML={{ __html: renderMarkdown(source) }}
    />
  );
}
