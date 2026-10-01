/**
 * Inline diff comment treatment (draft composer + persisted comment).
 */
import { MessageCircle, Trash2 } from 'lucide-react';
import { useLayoutEffect, useRef, useState, type ReactNode } from 'react';

import { cn } from '@/lib/utils';

import { isCommentSubmitShortcut } from './commentSubmitShortcut';

interface DiffCommentSecondaryAction {
  readonly label: string;
  readonly icon?: ReactNode;
  readonly allowEmpty?: boolean;
  readonly onAction: (text: string) => void;
}

interface DiffCommentAnnotationProps {
  kind: 'draft' | 'comment';
  rangeLabel: string;
  text: string;
  onTextChange?: (text: string) => void;
  onCancel: () => void;
  onComment: (text: string) => void;
  onDelete?: () => void;
  placeholder?: string;
  submitLabel?: string;
  pending?: boolean;
  secondaryAction?: DiffCommentSecondaryAction;
  focusOnMount?: boolean;
}

/** The shared inline comment treatment for file previews and diffs. */
export function DiffCommentAnnotation({
  kind,
  rangeLabel,
  text,
  onTextChange,
  onCancel,
  onComment,
  onDelete,
  placeholder = 'Add a comment…',
  submitLabel = 'Comment',
  pending = false,
  secondaryAction,
  focusOnMount = true,
}: DiffCommentAnnotationProps) {
  const [localDraftText, setLocalDraftText] = useState('');
  const displayedText = kind === 'draft' && !onTextChange ? localDraftText : text;
  const trimmedText = displayedText.trim();
  const textareaRef = useRef<HTMLTextAreaElement | null>(null);

  useLayoutEffect(() => {
    if (kind !== 'draft' || !focusOnMount) return;
    const frame = window.requestAnimationFrame(() => {
      textareaRef.current?.focus({ preventScroll: true });
    });
    return () => window.cancelAnimationFrame(frame);
  }, [focusOnMount, kind]);

  if (kind === 'comment') {
    return (
      <div
        data-diff-comment-annotation
        className="group/comment flex min-w-0 items-start gap-2.5 border-l-2 border-mira-blue/55 bg-mira-blue/[0.045] px-3 py-2.5 font-sans text-foreground"
        contentEditable={false}
        onPointerDown={(event) => event.stopPropagation()}
      >
        <MessageCircle className="mt-0.5 size-3.5 shrink-0 text-mira-blue/70" aria-hidden="true" />
        <p className="min-w-0 flex-1 whitespace-pre-wrap text-sm leading-5">{displayedText}</p>
        {onDelete ? (
          <span className="-my-1 -mr-1 flex shrink-0 opacity-0 transition-opacity group-hover/comment:opacity-100 focus-within:opacity-100 max-sm:opacity-100">
            <button
              type="button"
              aria-label="Delete comment"
              onClick={onDelete}
              className="rounded-md p-1.5 text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
            >
              <Trash2 className="size-3" />
            </button>
          </span>
        ) : null}
      </div>
    );
  }

  return (
    <div
      data-diff-comment-annotation
      className="px-3 py-2 font-sans text-foreground"
      contentEditable={false}
      onPointerDown={(event) => event.stopPropagation()}
    >
      <textarea
        ref={textareaRef}
        autoFocus={focusOnMount}
        value={displayedText}
        placeholder={placeholder}
        aria-label={`Comment on lines ${rangeLabel}`}
        rows={2}
        onChange={(event) => (onTextChange ?? setLocalDraftText)(event.target.value)}
        onFocus={(event) => {
          const end = event.currentTarget.value.length;
          event.currentTarget.setSelectionRange(end, end);
        }}
        onKeyDown={(event) => {
          if (event.key === 'Escape') {
            event.preventDefault();
            onCancel();
          }
          if (isCommentSubmitShortcut(event, trimmedText, pending)) {
            event.preventDefault();
            onComment(trimmedText);
          }
        }}
        className={cn(
          'w-full resize-none rounded-md border border-border bg-background px-2 py-1.5',
          'text-[13px] leading-5 outline-none focus:border-foreground/30',
        )}
      />
      <div className="mt-1.5 flex items-center gap-1">
        <span className="mr-auto text-[10px] text-muted-foreground/70">⌘/Ctrl Enter to send</span>
        <button
          type="button"
          onClick={onCancel}
          className="rounded-md px-2 py-1 text-xs text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
        >
          Cancel
        </button>
        {secondaryAction ? (
          <button
            type="button"
            disabled={!secondaryAction.allowEmpty && !trimmedText}
            onClick={() => secondaryAction.onAction(trimmedText)}
            className="flex items-center gap-1 rounded-md border border-border px-2 py-1 text-xs transition-colors hover:bg-accent disabled:opacity-40"
          >
            {secondaryAction.icon}
            {secondaryAction.label}
          </button>
        ) : null}
        <button
          type="button"
          disabled={pending || !trimmedText}
          onClick={() => onComment(trimmedText)}
          className="rounded-md bg-foreground px-2 py-1 text-xs font-medium text-background disabled:opacity-40"
        >
          {submitLabel}
        </button>
      </div>
    </div>
  );
}
