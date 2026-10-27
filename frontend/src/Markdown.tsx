import { useEffect, useState, type ReactNode } from "react";
import { api, type Attachment } from "./api";

const imageCache = new Map<string, string>();

function isRemoteOrDataUrl(src: string): boolean {
  return (
    src.startsWith("http://") ||
    src.startsWith("https://") ||
    src.startsWith("data:") ||
    src.startsWith("blob:")
  );
}

export function LocalOrRemoteImage({
  src,
  alt,
  className,
}: {
  src: string;
  alt: string;
  className?: string;
}) {
  const [resolved, setResolved] = useState<string>(() => {
    if (isRemoteOrDataUrl(src)) return src;
    return imageCache.get(src) ?? "";
  });
  const [failed, setFailed] = useState(false);

  useEffect(() => {
    setFailed(false);
    if (isRemoteOrDataUrl(src)) {
      setResolved(src);
      return;
    }
    const cached = imageCache.get(src);
    if (cached) {
      setResolved(cached);
      return;
    }
    let cancelled = false;
    api
      .readImageDataUrl(src)
      .then((dataUrl) => {
        if (cancelled) return;
        imageCache.set(src, dataUrl);
        setResolved(dataUrl);
      })
      .catch(() => {
        if (!cancelled) setFailed(true);
      });
    return () => {
      cancelled = true;
    };
  }, [src]);

  if (failed) {
    return (
      <span className="md-img-fallback" title={src}>
        [图片: {alt || src}]
      </span>
    );
  }
  if (!resolved) {
    return (
      <span className="md-img-loading" title={src}>
        加载图片 {alt || ""}…
      </span>
    );
  }
  return (
    <img
      src={resolved}
      alt={alt}
      className={className ?? "md-image"}
      loading="lazy"
      onError={() => setFailed(true)}
    />
  );
}

function renderInline(text: string, keyPrefix: string): ReactNode[] {
  const out: ReactNode[] = [];
  // Matches: ![alt](src), [label](href), `code`, **bold**, *italic*
  const pattern =
    /!\[([^\]]*)\]\(([^)\s]+)(?:\s+"[^"]*")?\)|\[([^\]]+)\]\((https?:\/\/[^)\s]+)\)|`([^`]+)`|\*\*([^*]+)\*\*|\*([^*\n]+)\*/g;
  let lastIndex = 0;
  let match: RegExpExecArray | null;
  let idx = 0;

  while ((match = pattern.exec(text)) !== null) {
    if (match.index > lastIndex) {
      out.push(text.slice(lastIndex, match.index));
    }
    const k = `${keyPrefix}-i${idx++}`;
    if (match[1] !== undefined && match[2] !== undefined) {
      out.push(<LocalOrRemoteImage key={k} alt={match[1]} src={match[2]} />);
    } else if (match[3] !== undefined && match[4] !== undefined) {
      out.push(
        <a
          key={k}
          href={match[4]}
          target="_blank"
          rel="noreferrer"
          className="md-link"
        >
          {match[3]}
        </a>,
      );
    } else if (match[5] !== undefined) {
      out.push(
        <code key={k} className="md-inline-code">
          {match[5]}
        </code>,
      );
    } else if (match[6] !== undefined) {
      out.push(<strong key={k}>{match[6]}</strong>);
    } else if (match[7] !== undefined) {
      out.push(<em key={k}>{match[7]}</em>);
    }
    lastIndex = pattern.lastIndex;
  }

  if (lastIndex < text.length) {
    out.push(text.slice(lastIndex));
  }
  return out.length > 0 ? out : [text];
}

function parseTableRow(line: string): string[] {
  const trimmed = line.trim().replace(/^\||\|$/g, "");
  return trimmed.split("|").map((c) => c.trim());
}

function isTableSeparator(line: string): boolean {
  const trimmed = line.trim();
  if (!trimmed.includes("|") || !trimmed.includes("-")) return false;
  const cells = parseTableRow(trimmed);
  return cells.length > 0 && cells.every((c) => /^:?-{2,}:?$/.test(c));
}

function renderBlocks(text: string): ReactNode[] {
  const lines = text.replace(/\r\n/g, "\n").split("\n");
  const blocks: ReactNode[] = [];
  let i = 0;
  let bIdx = 0;

  while (i < lines.length) {
    const line = lines[i];

    // Skip blank lines
    if (line.trim() === "") {
      i++;
      continue;
    }

    // Fenced code block ```lang
    const fence = line.match(/^```(\S*)\s*$/);
    if (fence) {
      const lang = fence[1] || "";
      const codeLines: string[] = [];
      i++;
      while (i < lines.length && !/^```\s*$/.test(lines[i])) {
        codeLines.push(lines[i]);
        i++;
      }
      if (i < lines.length) i++; // skip closing ```
      const codeText = codeLines.join("\n");
      const key = `b-${bIdx++}`;
      blocks.push(
        <div key={key} className="md-code-block">
          {lang && (
            <div className="md-code-header" aria-hidden="true">
              <span>{lang}</span>
            </div>
          )}
          <pre>
            <code>{codeText}</code>
          </pre>
        </div>,
      );
      continue;
    }

    // Heading #..######
    const heading = line.match(/^(#{1,6})\s+(.+)$/);
    if (heading) {
      const level = heading[1].length;
      const content = heading[2];
      const key = `b-${bIdx++}`;
      const children = renderInline(content, key);
      if (level === 1) blocks.push(<h3 key={key} className="md-h1">{children}</h3>);
      else if (level === 2) blocks.push(<h4 key={key} className="md-h2">{children}</h4>);
      else blocks.push(<h5 key={key} className="md-h3">{children}</h5>);
      i++;
      continue;
    }

    // Horizontal rule
    if (/^(-{3,}|\*{3,}|_{3,})\s*$/.test(line.trim())) {
      blocks.push(<hr key={`b-${bIdx++}`} className="md-hr" />);
      i++;
      continue;
    }

    // Table
    if (
      line.includes("|") &&
      i + 1 < lines.length &&
      isTableSeparator(lines[i + 1])
    ) {
      const headers = parseTableRow(line);
      i += 2;
      const rows: string[][] = [];
      while (
        i < lines.length &&
        lines[i].trim() !== "" &&
        lines[i].includes("|")
      ) {
        rows.push(parseTableRow(lines[i]));
        i++;
      }
      const key = `b-${bIdx++}`;
      blocks.push(
        <div key={key} className="md-table-wrap">
          <table className="md-table">
            <thead>
              <tr>
                {headers.map((h, hIdx) => (
                  <th key={`${key}-th-${hIdx}`}>
                    {renderInline(h, `${key}-th-${hIdx}`)}
                  </th>
                ))}
              </tr>
            </thead>
            <tbody>
              {rows.map((r, rIdx) => (
                <tr key={`${key}-tr-${rIdx}`}>
                  {r.map((c, cIdx) => (
                    <td key={`${key}-td-${rIdx}-${cIdx}`}>
                      {renderInline(c, `${key}-td-${rIdx}-${cIdx}`)}
                    </td>
                  ))}
                </tr>
              ))}
            </tbody>
          </table>
        </div>,
      );
      continue;
    }

    // Blockquote >
    if (/^>\s?/.test(line)) {
      const qLines: string[] = [];
      while (i < lines.length && /^>\s?/.test(lines[i])) {
        qLines.push(lines[i].replace(/^>\s?/, ""));
        i++;
      }
      const key = `b-${bIdx++}`;
      blocks.push(
        <blockquote key={key} className="md-quote">
          {renderInline(qLines.join("\n"), key)}
        </blockquote>,
      );
      continue;
    }

    // Unordered list - / *
    if (/^\s*[-*+]\s+/.test(line)) {
      const items: string[] = [];
      while (i < lines.length && /^\s*[-*+]\s+/.test(lines[i])) {
        items.push(lines[i].replace(/^\s*[-*+]\s+/, ""));
        i++;
      }
      const key = `b-${bIdx++}`;
      blocks.push(
        <ul key={key} className="md-list">
          {items.map((it, idx) => (
            <li key={`${key}-li-${idx}`}>
              {renderInline(it, `${key}-li-${idx}`)}
            </li>
          ))}
        </ul>,
      );
      continue;
    }

    // Ordered list 1.
    if (/^\s*\d+\.\s+/.test(line)) {
      const items: string[] = [];
      while (i < lines.length && /^\s*\d+\.\s+/.test(lines[i])) {
        items.push(lines[i].replace(/^\s*\d+\.\s+/, ""));
        i++;
      }
      const key = `b-${bIdx++}`;
      blocks.push(
        <ol key={key} className="md-list">
          {items.map((it, idx) => (
            <li key={`${key}-oli-${idx}`}>
              {renderInline(it, `${key}-oli-${idx}`)}
            </li>
          ))}
        </ol>,
      );
      continue;
    }

    // Regular paragraph (combine consecutive normal lines)
    const pLines: string[] = [line];
    i++;
    while (
      i < lines.length &&
      lines[i].trim() !== "" &&
      !/^```/.test(lines[i]) &&
      !/^#{1,6}\s+/.test(lines[i]) &&
      !/^>\s?/.test(lines[i]) &&
      !/^\s*[-*+]\s+/.test(lines[i]) &&
      !/^\s*\d+\.\s+/.test(lines[i]) &&
      !(
        lines[i].includes("|") &&
        i + 1 < lines.length &&
        isTableSeparator(lines[i + 1])
      )
    ) {
      pLines.push(lines[i]);
      i++;
    }
    const key = `b-${bIdx++}`;
    blocks.push(
      <p key={key} className="md-p">
        {renderInline(pLines.join("\n"), key)}
      </p>,
    );
  }

  return blocks;
}

export function MessageContent({
  content,
  attachments,
}: {
  content: string;
  attachments?: Attachment[];
}) {
  const imageAttachments = (attachments ?? []).filter((a) =>
    a.mime.startsWith("image/"),
  );
  const otherAttachments = (attachments ?? []).filter(
    (a) => !a.mime.startsWith("image/"),
  );

  return (
    <div className="md-body">
      {content && renderBlocks(content)}
      {imageAttachments.length > 0 && (
        <div className="msg-images">
          {imageAttachments.map((att) => (
            <figure key={att.id} className="msg-image-card">
              <LocalOrRemoteImage
                src={att.stored_path}
                alt={att.original_name}
                className="msg-attachment-img"
              />
              <figcaption>{att.original_name}</figcaption>
            </figure>
          ))}
        </div>
      )}
      {otherAttachments.length > 0 && (
        <div className="msg-files">
          {otherAttachments.map((att) => (
            <span key={att.id} className="msg-file-pill">
              📎 {att.original_name}
            </span>
          ))}
        </div>
      )}
    </div>
  );
}
