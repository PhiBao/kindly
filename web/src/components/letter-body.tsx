"use client";

/**
 * Letters carry the payment link as plain text (they are also sent by email,
 * where a link has to survive as text). In the UI we make the link clickable
 * and quiet, so a letter still reads like a letter.
 */
export function LetterBody({ children }: { children: string }) {
  const parts = children.split(/(https?:\/\/\S+)/g);
  return (
    <div className="letter-body whitespace-pre-wrap">
      {parts.map((part, index) =>
        /^https?:\/\//.test(part) ? (
          <a
            key={index}
            href={part}
            target="_blank"
            rel="noreferrer"
            className="break-all underline decoration-dotted underline-offset-2"
            style={{ color: "var(--color-stamp)" }}
          >
            {part.replace(/^https?:\/\//, "")}
          </a>
        ) : (
          <span key={index}>{part}</span>
        ),
      )}
    </div>
  );
}

