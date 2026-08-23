import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { cn } from "../lib/utils";

/**
 * An agent's words, rendered as the Markdown they are written in.
 *
 * Agents answer in Markdown — headings, lists, bold, the odd table — and the
 * chat used to show the raw asterisks. This renders the text with GFM
 * (lists, tables, strikethrough, task lists) and **no raw HTML**: whatever
 * an agent writes between angle brackets is shown as text, never interpreted.
 * Links open in a new tab and never carry the opener.
 *
 * Styling is local rather than a typography plugin: a chat bubble wants tight
 * spacing and the bubble's own colours, not an article's.
 */
export function Markdown({ text, className }: { text: string; className?: string }) {
  return (
    <div className={cn("md min-w-0 break-words", className)}>
      <ReactMarkdown
        remarkPlugins={[remarkGfm]}
        skipHtml
        components={{
          a: ({ href, children }) => (
            <a
              href={href}
              target="_blank"
              rel="noopener noreferrer"
              className="underline underline-offset-2 hover:opacity-80"
            >
              {children}
            </a>
          ),
          p: ({ children }) => <p className="my-1.5 first:mt-0 last:mb-0">{children}</p>,
          ul: ({ children }) => <ul className="my-1.5 list-disc space-y-0.5 pl-5">{children}</ul>,
          ol: ({ children }) => <ol className="my-1.5 list-decimal space-y-0.5 pl-5">{children}</ol>,
          li: ({ children }) => <li className="[&>p]:my-0">{children}</li>,
          h1: ({ children }) => <h3 className="mt-3 mb-1 text-base font-semibold first:mt-0">{children}</h3>,
          h2: ({ children }) => <h3 className="mt-3 mb-1 text-[15px] font-semibold first:mt-0">{children}</h3>,
          h3: ({ children }) => <h4 className="mt-2.5 mb-1 text-sm font-semibold first:mt-0">{children}</h4>,
          h4: ({ children }) => <h5 className="mt-2 mb-0.5 text-sm font-semibold first:mt-0">{children}</h5>,
          h5: ({ children }) => <h6 className="mt-2 mb-0.5 text-sm font-medium first:mt-0">{children}</h6>,
          h6: ({ children }) => <h6 className="mt-2 mb-0.5 text-sm font-medium first:mt-0">{children}</h6>,
          strong: ({ children }) => <strong className="font-semibold">{children}</strong>,
          blockquote: ({ children }) => (
            <blockquote className="my-1.5 border-l-2 border-current/30 pl-3 opacity-90">{children}</blockquote>
          ),
          hr: () => <hr className="my-2 border-current/20" />,
          code: ({ className: cls, children }) => {
            const block = /language-/.test(cls ?? "") || String(children).includes("\n");
            return block ? (
              <code className="mono block whitespace-pre text-[12.5px]">{children}</code>
            ) : (
              <code className="mono rounded bg-current/10 px-1 py-0.5 text-[12.5px]">{children}</code>
            );
          },
          pre: ({ children }) => (
            <pre className="my-2 overflow-x-auto rounded-lg bg-current/10 p-3 text-[12.5px]">{children}</pre>
          ),
          table: ({ children }) => (
            <div className="my-2 overflow-x-auto">
              <table className="w-full border-collapse text-[13px]">{children}</table>
            </div>
          ),
          th: ({ children }) => (
            <th className="border-b border-current/20 px-2 py-1 text-left font-semibold">{children}</th>
          ),
          td: ({ children }) => <td className="border-b border-current/10 px-2 py-1 align-top">{children}</td>,
        }}
      >
        {text}
      </ReactMarkdown>
    </div>
  );
}
