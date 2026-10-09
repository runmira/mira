import { Popover, PopoverContent, PopoverTrigger } from '@/components/ui/popover';
import { cn } from '@/lib/utils';
import { AnimatePresence, m, useReducedMotion } from 'framer-motion';
import {
  ChevronDown,
  CornerDownRight,
  File as FileIcon,
  GripVertical,
  MoreHorizontal,
  PenLine,
  ArrowDown as PhArrowDown,
  ArrowUp as PhArrowUp,
  Trash,
} from 'lucide-react';
import { useId, useRef, useState } from 'react';
import { Dialog, DialogContent, DialogDescription, DialogTitle } from '../ui/dialog';
import { parseSentAttachments } from './attachments';
import { MenuButton } from './ComposerMenus';
import { QueuedComposerMessage } from './types';
export function QueuedMessageStack({
  items,
  onRemove,
  onSteer,
  onEdit,
  onReorder,
  canSteer = true,
}: {
  items: QueuedComposerMessage[];
  canSteer?: boolean;
  onRemove?: (id: string) => void;
  onSteer?: (id: string) => void;
  onEdit?: (item: QueuedComposerMessage, text: string) => Promise<void>;
  onReorder?: (id: string, beforeId: string | null) => Promise<void>;
}) {
  const [expanded, setExpanded] = useState(false);
  const [editing, setEditing] = useState<QueuedComposerMessage | null>(null);
  const [editText, setEditText] = useState('');
  const [saving, setSaving] = useState(false);
  const [moving, setMoving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [editError, setEditError] = useState<string | null>(null);
  const [dropTarget, setDropTarget] = useState<string | null>(null);
  const dragId = useRef<string | null>(null);
  const armedId = useRef<string | null>(null);
  const listId = useId();
  const reduceMotion = useReducedMotion();
  const editable = (item: QueuedComposerMessage) =>
    !item.steering && !item.error && !!item.fingerprint;
  async function move(id: string, beforeId: string | null) {
    if (!onReorder || moving || saving) return;
    setMoving(true);
    setError(null);
    try {
      await onReorder(id, beforeId);
    } catch (error) {
      setError(error instanceof Error ? error.message : 'Could not reorder the queue.');
    } finally {
      setMoving(false);
    }
  }
  async function save() {
    if (!editing || !onEdit || saving) return;
    const parsed = parseSentAttachments(editing.text);
    const prefix = parsed.attachments.length
      ? editing.text.slice(0, editing.text.length - parsed.text.length)
      : '';
    setSaving(true);
    setEditError(null);
    try {
      await onEdit(editing, prefix + editText);
      setEditing(null);
    } catch (error) {
      setEditError(error instanceof Error ? error.message : 'Could not save this edit.');
    } finally {
      setSaving(false);
    }
  }
  if (items.length === 0 && !editing) return null;
  const shown = expanded ? items : items.slice(0, 4);
  const extra = items.length - shown.length;
  return (
    <>
      {items.length > 0 && (
        <m.div
          layout
          className="-mb-3 w-full max-w-3xl px-7 sm:px-8"
          initial={{ opacity: 0, y: reduceMotion ? 0 : 8 }}
          animate={{ opacity: 1, y: 0 }}
          exit={{ opacity: 0, y: 8 }}
          transition={{ duration: reduceMotion ? 0 : 0.18, ease: [0.4, 0, 0.2, 1] }}
        >
          <div className="overflow-hidden rounded-t-[18px] rounded-b-none border border-b-0 border-border/55 bg-white pb-4 pt-1.5 shadow-[0_8px_24px_-22px_rgba(15,23,42,0.45)] dark:border-fg/[0.055] dark:bg-secondary/85 dark:shadow-[0_10px_28px_-24px_rgba(0,0,0,0.75)] backdrop-blur-xl">
            <div
              id={listId}
              role="list"
              aria-label="Queued messages"
              className="max-h-64 overflow-y-auto"
            >
              <AnimatePresence initial={false}>
                {shown.map((item, index) => (
                  <m.div
                    layout={!reduceMotion}
                    key={item.id}
                    initial={{ opacity: 0, height: 0, y: 6 }}
                    animate={{ opacity: 1, height: 'auto', y: 0 }}
                    exit={{ opacity: 0, height: 0, y: -4 }}
                    transition={{ duration: reduceMotion ? 0 : 0.18, ease: [0.4, 0, 0.2, 1] }}
                    role="listitem"
                    draggable={!!onReorder && editable(item) && !moving && !saving}
                    onDragStartCapture={(event) => {
                      if (armedId.current !== item.id) {
                        event.preventDefault();
                        return;
                      }
                      dragId.current = item.id;
                      event.dataTransfer.setData('application/x-mira-queued-message', item.id);
                      event.dataTransfer.effectAllowed = 'move';
                    }}
                    onDragOver={(event) => {
                      if (!dragId.current || dragId.current === item.id || !editable(item)) return;
                      event.preventDefault();
                      event.dataTransfer.dropEffect = 'move';
                      setDropTarget(item.id);
                    }}
                    onDrop={(event) => {
                      const id = dragId.current;
                      if (!id || id === item.id || !editable(item)) return;
                      event.preventDefault();
                      dragId.current = null;
                      armedId.current = null;
                      setDropTarget(null);
                      void move(id, item.id);
                    }}
                    onDragEndCapture={() => {
                      dragId.current = null;
                      armedId.current = null;
                      setDropTarget(null);
                    }}
                    className={cn(
                      'flex min-w-0 items-center gap-2 px-3.5 py-1.5 text-[13px]',
                      dropTarget === item.id && 'ring-1 ring-inset ring-mira-blue',
                    )}
                  >
                    {onReorder && editable(item) ? (
                      <button
                        type="button"
                        aria-label="Drag to reorder queued message"
                        title="Drag to reorder; use the options menu to move with the keyboard"
                        disabled={moving || saving}
                        onPointerDown={() => {
                          armedId.current = item.id;
                        }}
                        onPointerUp={() => {
                          armedId.current = null;
                        }}
                        className="flex size-5 shrink-0 cursor-grab items-center justify-center text-muted-foreground/70 active:cursor-grabbing"
                      >
                        <GripVertical className="size-3.5" />
                      </button>
                    ) : (
                      <CornerDownRight className="size-3.5 shrink-0 text-muted-foreground/55" />
                    )}
                    <QueuedThumb item={item} />
                    <span className="min-w-0 flex-1 truncate text-foreground/90">
                      {queueTitle(item)}
                      {item.error && (
                        <span
                          role="status"
                          className="ml-2 text-xs text-muted-foreground"
                          title={item.error}
                        >
                          {item.error}
                        </span>
                      )}
                    </span>
                    <button
                      type="button"
                      onPointerDown={(e) => e.preventDefault()}
                      disabled={item.steering || !!item.error || !canSteer || saving || moving}
                      onClick={() => onSteer?.(item.id)}
                      className="inline-flex shrink-0 items-center gap-1 rounded-md px-1.5 py-1 text-muted-foreground transition-colors hover:bg-fg/[0.06] hover:text-foreground"
                      title={
                        canSteer
                          ? 'Send as input to the active turn'
                          : 'Steering is unavailable; this message will run after the current turn'
                      }
                    >
                      <CornerDownRight className="size-3.5" />
                      <span>{item.steering ? 'Sending…' : 'Steer'}</span>
                    </button>
                    <button
                      type="button"
                      onPointerDown={(e) => e.preventDefault()}
                      onClick={() => onRemove?.(item.id)}
                      disabled={item.steering || saving || moving}
                      aria-label="Remove queued message"
                      className="inline-flex size-7 shrink-0 items-center justify-center rounded-md text-muted-foreground/80 transition-colors hover:bg-fg/[0.06] hover:text-foreground"
                    >
                      <Trash className="size-3.5" />
                    </button>
                    {!item.steering && (
                      <QueuedMessageMenu
                        item={item}
                        onEdit={
                          editable(item) && onEdit && !saving && !moving
                            ? (item) => {
                                setEditing(item);
                                setEditText(parseSentAttachments(item.text).text);
                                setEditError(null);
                              }
                            : undefined
                        }
                        onMoveUp={
                          index > 0 &&
                          editable(item) &&
                          editable(items[index - 1]) &&
                          onReorder &&
                          !moving &&
                          !saving
                            ? () => void move(item.id, items[index - 1].id)
                            : undefined
                        }
                        onMoveDown={
                          index < items.length - 1 &&
                          editable(item) &&
                          editable(items[index + 1]) &&
                          onReorder &&
                          !moving &&
                          !saving
                            ? () => void move(item.id, items[index + 2]?.id ?? null)
                            : undefined
                        }
                        onRemove={saving || moving ? undefined : onRemove}
                      />
                    )}
                  </m.div>
                ))}
              </AnimatePresence>
            </div>
            {items.length > 4 && (
              <button
                type="button"
                aria-expanded={expanded}
                aria-controls={listId}
                onClick={() => setExpanded((value) => !value)}
                className="flex w-full items-center gap-1.5 px-10 py-1 text-left text-[12px] text-muted-foreground hover:text-foreground"
              >
                {expanded ? 'Show fewer messages' : `Show ${extra} more queued`}
                <ChevronDown
                  className={cn('size-3 transition-transform', expanded && 'rotate-180')}
                />
              </button>
            )}
            {error && (
              <p role="alert" className="px-4 py-1 text-xs text-destructive">
                {error}
              </p>
            )}
          </div>
        </m.div>
      )}
      <Dialog
        open={!!editing}
        onOpenChange={(open) => {
          if (!open && !saving) setEditing(null);
        }}
      >
        <DialogContent
          onEscapeKeyDown={(event) => {
            if (saving) event.preventDefault();
          }}
          onPointerDownOutside={(event) => {
            if (saving) event.preventDefault();
          }}
        >
          <DialogTitle>Edit queued message</DialogTitle>
          <DialogDescription>
            The original stays in the queue until your changes are saved.
          </DialogDescription>
          <textarea
            aria-label="Queued message text"
            value={editText}
            onChange={(event) => setEditText(event.target.value)}
            disabled={saving}
            className="min-h-32 w-full resize-y rounded-lg border border-border bg-background p-3 text-[13px] outline-none focus:ring-1 focus:ring-mira-blue"
          />
          {editing && (
            <div className="flex items-center gap-2 text-xs text-muted-foreground">
              <QueuedThumb item={editing} />
              {editing.images?.length
                ? `${editing.images.length} image attachment${editing.images.length === 1 ? '' : 's'} retained`
                : parseSentAttachments(editing.text)
                    .attachments.map((file) => file.filename)
                    .join(', ')}
            </div>
          )}
          {editError && (
            <p role="alert" className="text-xs text-destructive">
              {editError}
            </p>
          )}
          <div className="flex justify-end gap-2">
            <button
              type="button"
              disabled={saving}
              onClick={() => setEditing(null)}
              className="rounded-md px-3 py-1.5 text-[13px] hover:bg-secondary"
            >
              Cancel
            </button>
            <button
              type="button"
              disabled={
                saving ||
                (!editText.trim() &&
                  !editing?.images?.length &&
                  !parseSentAttachments(editing?.text ?? '').attachments.length)
              }
              onClick={() => void save()}
              className="rounded-md bg-primary px-3 py-1.5 text-[13px] text-primary-foreground disabled:opacity-50"
            >
              {saving ? 'Saving…' : 'Save changes'}
            </button>
          </div>
        </DialogContent>
      </Dialog>
    </>
  );
}

export function queueTitle(item: QueuedComposerMessage) {
  const { text, attachments } = parseSentAttachments(item.text);
  const prose = text.replace(/\s+/g, ' ').trim();
  if (prose) return prose;
  const n = item.images?.length ?? 0;
  if (n) return n === 1 ? 'Image attachment' : `${n} image attachments`;
  if (attachments.length)
    return attachments.length === 1
      ? attachments[0].filename
      : `${attachments.length} file attachments`;
  return 'Queued message';
}

export function QueuedThumb({ item }: { item: QueuedComposerMessage }) {
  const first = item.images?.[0];
  if (first) {
    return (
      <img
        src={`data:${first.media_type};base64,${first.data}`}
        alt=""
        className="size-7 shrink-0 rounded-md object-cover ring-1 ring-fg/[0.08]"
        draggable={false}
      />
    );
  }
  const attachments = parseSentAttachments(item.text).attachments;
  if (!attachments.length) return null;
  return (
    <span
      title={attachments.map((file) => file.filename).join(', ')}
      className="flex size-7 shrink-0 items-center justify-center rounded-md bg-fg/[0.06] text-muted-foreground"
    >
      <FileIcon className="size-3.5" />
    </span>
  );
}

export function QueuedMessageMenu({
  item,
  onEdit,
  onRemove,
  onMoveUp,
  onMoveDown,
}: {
  item: QueuedComposerMessage;
  onEdit?: (item: QueuedComposerMessage) => void;
  onRemove?: (id: string) => void;
  onMoveUp?: () => void;
  onMoveDown?: () => void;
}) {
  const [open, setOpen] = useState(false);
  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <button
          type="button"
          aria-label="Queued message options"
          className="inline-flex size-7 shrink-0 items-center justify-center rounded-md text-muted-foreground/70 transition-colors hover:bg-fg/[0.06] hover:text-foreground"
          title="Queued message options"
        >
          <MoreHorizontal className="size-3.5" />
        </button>
      </PopoverTrigger>
      <PopoverContent className="w-44 p-1.5" align="end">
        {onMoveUp && (
          <MenuButton
            icon={<PhArrowUp className="size-3.5" />}
            label="Move up"
            onClick={() => {
              setOpen(false);
              onMoveUp();
            }}
          />
        )}
        {onMoveDown && (
          <MenuButton
            icon={<PhArrowDown className="size-3.5" />}
            label="Move down"
            onClick={() => {
              setOpen(false);
              onMoveDown();
            }}
          />
        )}
        {onEdit && (
          <MenuButton
            icon={<PenLine className="size-3.5" />}
            label="Edit"
            hint="Keep its place in the queue"
            onClick={() => {
              setOpen(false);
              onEdit?.(item);
            }}
          />
        )}
        <MenuButton
          icon={<Trash className="size-3.5" />}
          label="Remove"
          hint="Cancel this queued message"
          onClick={() => {
            setOpen(false);
            onRemove?.(item.id);
          }}
        />
      </PopoverContent>
    </Popover>
  );
}
