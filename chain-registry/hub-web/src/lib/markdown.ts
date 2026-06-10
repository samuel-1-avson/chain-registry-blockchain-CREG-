/** Minimal markdown → HTML for hub journey copy (headings, lists, tables, links). */
export function renderMarkdown(source: string): string {
  const lines = source.replace(/\r\n/g, "\n").split("\n");
  const html: string[] = [];
  let inList = false;
  let tableRows: string[][] = [];
  let tableHeader: string[] | null = null;

  const flushList = () => {
    if (inList) {
      html.push("</ul>");
      inList = false;
    }
  };

  const flushTable = () => {
    if (tableRows.length === 0 && !tableHeader) return;
    html.push("<table><thead><tr>");
    for (const cell of tableHeader ?? tableRows[0] ?? []) {
      html.push(`<th>${inline(cell)}</th>`);
    }
    html.push("</tr></thead><tbody>");
    const bodyRows = tableHeader ? tableRows : tableRows.slice(1);
    for (const row of bodyRows) {
      html.push("<tr>");
      for (const cell of row) {
        html.push(`<td>${inline(cell)}</td>`);
      }
      html.push("</tr>");
    }
    html.push("</tbody></table>");
    tableRows = [];
    tableHeader = null;
  };

  const inline = (text: string): string => {
    return escapeHtml(text)
      .replace(/\[([^\]]+)\]\(([^)]+)\)/g, '<a href="$2">$1</a>')
      .replace(/\*\*([^*]+)\*\*/g, "<strong>$1</strong>")
      .replace(/`([^`]+)`/g, "<code>$1</code>");
  };

  for (const raw of lines) {
    const line = raw.trimEnd();

    if (line.startsWith("|")) {
      flushList();
      const cells = line
        .split("|")
        .slice(1, -1)
        .map((c) => c.trim());
      if (cells.every((c) => /^[-:]+$/.test(c))) {
        tableHeader = tableRows[0] ?? null;
        tableRows = [];
        continue;
      }
      tableRows.push(cells);
      continue;
    }

    flushTable();

    if (line.startsWith("# ")) {
      flushList();
      html.push(`<h1>${inline(line.slice(2))}</h1>`);
      continue;
    }
    if (line.startsWith("## ")) {
      flushList();
      html.push(`<h2>${inline(line.slice(3))}</h2>`);
      continue;
    }
    if (line.startsWith("### ")) {
      flushList();
      html.push(`<h3>${inline(line.slice(4))}</h3>`);
      continue;
    }
    if (line.startsWith("- ")) {
      if (!inList) {
        html.push("<ul>");
        inList = true;
      }
      html.push(`<li>${inline(line.slice(2))}</li>`);
      continue;
    }
    if (/^\d+\.\s/.test(line)) {
      flushList();
      html.push(`<p>${inline(line)}</p>`);
      continue;
    }
    if (line === "") {
      flushList();
      continue;
    }

    flushList();
    html.push(`<p>${inline(line)}</p>`);
  }

  flushList();
  flushTable();
  return html.join("");
}

function escapeHtml(text: string): string {
  return text
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}
