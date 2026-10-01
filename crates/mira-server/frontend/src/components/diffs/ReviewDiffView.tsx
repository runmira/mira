/**
 * Pierre CodeView with inline review comments.
 *
 * Comments are `MiraReviewComment`s owned by the parent; the draft
 * (unsubmitted) composer lives here, grouped with persisted comments
 * per annotated line.
 */
import type {
  AnnotationSide,
  CodeViewDiffItem,
  CodeViewItem,
  DiffLineAnnotation,
  FileDiffMetadata,
  SelectedLineRange,
} from '@pierre/diffs';
import type { CodeViewHandle } from '@pierre/diffs/react';
import { useCallback, useMemo, useState, type ReactNode, type Ref } from 'react';

import { fnv1a32 } from '@/lib/diffRender';
import {
  buildMiraReviewComment,
  nextReviewCommentId,
  type MiraReviewComment,
} from '@/lib/reviewComment';
import { DiffCommentAnnotation } from './DiffCommentAnnotation';
import { StyledDiffCodeView, type StyledDiffCodeViewOptions } from './StyledDiffCodeView';

interface DiffCommentAnnotationEntry {
  id: string;
  kind: 'draft' | 'comment';
  range: SelectedLineRange;
  rangeLabel: string;
  text: string;
}

interface DiffCommentAnnotationGroup {
  entries: DiffCommentAnnotationEntry[];
}

type DiffCommentLineAnnotation = DiffLineAnnotation<DiffCommentAnnotationGroup>;
export type ReviewDiffViewHandle = CodeViewHandle<DiffCommentAnnotationGroup, undefined>;

function annotationSide(range: SelectedLineRange): AnnotationSide {
  return (range.endSide ?? range.side) === 'deletions' ? 'deletions' : 'additions';
}

function appendAnnotationEntry(
  annotations: ReadonlyArray<DiffCommentLineAnnotation>,
  range: SelectedLineRange,
  entry: DiffCommentAnnotationEntry,
): DiffCommentLineAnnotation[] {
  const side = annotationSide(range);
  const annotationIndex = annotations.findIndex(
    (annotation) => annotation.side === side && annotation.lineNumber === range.end,
  );
  if (annotationIndex < 0) {
    return [
      ...annotations,
      {
        side,
        lineNumber: range.end,
        metadata: { entries: [entry] },
      },
    ];
  }
  return annotations.map((annotation, index) =>
    index === annotationIndex
      ? {
          ...annotation,
          metadata: { entries: [...annotation.metadata.entries, entry] },
        }
      : annotation,
  );
}

export interface ReviewDiffFile {
  fileDiff: FileDiffMetadata;
  filePath: string;
  fileKey: string;
  fileVersion: number;
  collapsed: boolean;
}

interface ReviewDiffViewProps {
  codeViewKey: string;
  files: ReadonlyArray<ReviewDiffFile>;
  comments: ReadonlyArray<MiraReviewComment>;
  onAddComment: (comment: MiraReviewComment) => void;
  onUpdateCommentText: (id: string, text: string) => void;
  onRemoveComment: (id: string) => void;
  options: StyledDiffCodeViewOptions<DiffCommentAnnotationGroup>;
  viewerRef?: Ref<ReviewDiffViewHandle>;
  className?: string;
  renderCodeViewFooter?: () => ReactNode;
  unsafeCSSExtra?: string;
  renderHeaderMetadata?: (fileDiff: FileDiffMetadata) => ReactNode;
  renderHeaderFilenameSuffix: (fileDiff: FileDiffMetadata) => ReactNode;
  renderHeaderPrefix: (
    fileDiff: FileDiffMetadata,
    fileKey: string,
    collapsed: boolean,
  ) => ReactNode;
}

interface DiffSelectionContext {
  item: CodeViewItem<DiffCommentAnnotationGroup>;
}

export function ReviewDiffView({
  codeViewKey,
  files,
  comments,
  onAddComment,
  onUpdateCommentText,
  onRemoveComment,
  options,
  viewerRef,
  className,
  renderCodeViewFooter,
  unsafeCSSExtra,
  renderHeaderMetadata,
  renderHeaderFilenameSuffix,
  renderHeaderPrefix,
}: ReviewDiffViewProps) {
  const [selectedLines, setSelectedLines] = useState<{
    id: string;
    range: SelectedLineRange;
  } | null>(null);
  const [draft, setDraft] = useState<{
    fileKey: string;
    annotation: DiffCommentLineAnnotation;
  } | null>(null);
  const [draftText, setDraftText] = useState('');

  const filesByKey = useMemo(() => new Map(files.map((file) => [file.fileKey, file])), [files]);
  const items = useMemo<CodeViewDiffItem<DiffCommentAnnotationGroup>[]>(
    () =>
      files.map(({ fileDiff, filePath, fileKey, fileVersion, collapsed }) => {
        const persisted = comments
          .filter((comment) => comment.fileKey === fileKey && comment.filePath === filePath)
          .reduce<DiffCommentLineAnnotation[]>((annotations, comment) => {
            return appendAnnotationEntry(annotations, comment.range, {
              id: comment.id,
              kind: 'comment',
              range: comment.range,
              rangeLabel: comment.rangeLabel,
              text: comment.text,
            });
          }, []);
        const annotations =
          draft?.fileKey === fileKey ? [...persisted, draft.annotation] : persisted;
        return {
          id: fileKey,
          type: 'diff',
          fileDiff,
          annotations,
          collapsed,
          version: fnv1a32(
            `${fileVersion}:${collapsed ? '1' : '0'}:${annotations
              .flatMap((annotation) =>
                annotation.metadata.entries.map(
                  (entry) => `${entry.id}:${entry.rangeLabel}:${entry.text}`,
                ),
              )
              .join(':')}`,
          ),
        };
      }),
    [draft, files, comments],
  );

  const removeEntry = useCallback(
    (entryId: string) => {
      setSelectedLines(null);
      if (draft?.annotation.metadata.entries.some((entry) => entry.id === entryId)) {
        setDraft(null);
        setDraftText('');
      } else {
        onRemoveComment(entryId);
      }
    },
    [draft, onRemoveComment],
  );

  const submitEntry = useCallback(
    (entryId: string, text: string) => {
      const entry = draft?.annotation.metadata.entries.find(
        (candidate) => candidate.id === entryId,
      );
      const file = draft ? filesByKey.get(draft.fileKey) : undefined;
      if (!entry || !file) return;
      const built = buildMiraReviewComment({
        id: entry.id,
        filePath: file.filePath,
        fileKey: file.fileKey,
        fileDiff: file.fileDiff,
        range: entry.range,
      });
      if (!built) return;
      onAddComment({ ...built, text });
      setSelectedLines(null);
      setDraft(null);
      setDraftText('');
    },
    [draft, filesByKey, onAddComment],
  );

  const beginComment = useCallback(
    (range: SelectedLineRange | null, context: DiffSelectionContext) => {
      if (!range) return;
      const item = context.item;
      if (item.type !== 'diff') return;
      const file = filesByKey.get(item.id);
      if (!file) return;
      const id = nextReviewCommentId();
      const built = buildMiraReviewComment({
        id,
        filePath: file.filePath,
        fileKey: file.fileKey,
        fileDiff: file.fileDiff,
        range,
      });
      if (!built) return;
      setDraftText('');
      setDraft({
        fileKey: item.id,
        annotation: {
          side: annotationSide(range),
          lineNumber: range.end,
          metadata: {
            entries: [{ id, kind: 'draft', range, rangeLabel: built.rangeLabel, text: '' }],
          },
        },
      });
    },
    [filesByKey],
  );

  const hasOpenComment = draft !== null;
  return (
    <StyledDiffCodeView<DiffCommentAnnotationGroup>
      key={codeViewKey}
      {...(viewerRef ? { viewerRef } : {})}
      {...(className ? { className } : {})}
      {...(unsafeCSSExtra ? { unsafeCSSExtra } : {})}
      {...(renderHeaderMetadata
        ? {
            renderHeaderMetadata: (item: CodeViewItem<DiffCommentAnnotationGroup>) =>
              item.type === 'diff' ? renderHeaderMetadata(item.fileDiff) : null,
          }
        : {})}
      {...(renderCodeViewFooter ? { renderCodeViewFooter } : {})}
      items={items}
      selectedLines={selectedLines}
      onSelectedLinesChange={setSelectedLines}
      options={{
        ...options,
        enableGutterUtility: !hasOpenComment,
        enableLineSelection: !hasOpenComment,
        onGutterUtilityClick: beginComment,
      }}
      renderHeaderFilenameSuffix={(item) =>
        item.type === 'diff' ? renderHeaderFilenameSuffix(item.fileDiff) : null
      }
      renderHeaderPrefix={(item) =>
        item.type === 'diff'
          ? renderHeaderPrefix(item.fileDiff, item.id, item.collapsed === true)
          : null
      }
      renderAnnotation={(annotation) => {
        const hasDraft = annotation.metadata.entries.some((entry) => entry.kind === 'draft');
        return (
          <div
            className={hasDraft ? 'py-1' : 'divide-y divide-border/30 border-y border-border/30'}
          >
            {annotation.metadata.entries.map((entry) =>
              entry.kind === 'draft' ? (
                <DiffCommentAnnotation
                  key={entry.id}
                  kind="draft"
                  rangeLabel={entry.rangeLabel}
                  text={draftText}
                  onTextChange={setDraftText}
                  onCancel={() => removeEntry(entry.id)}
                  onComment={(text) => submitEntry(entry.id, text)}
                  onDelete={() => removeEntry(entry.id)}
                />
              ) : (
                <DiffCommentAnnotation
                  key={entry.id}
                  kind="comment"
                  rangeLabel={entry.rangeLabel}
                  text={entry.text}
                  onCancel={() => removeEntry(entry.id)}
                  onComment={(text) => onUpdateCommentText(entry.id, text)}
                  onDelete={() => removeEntry(entry.id)}
                />
              ),
            )}
          </div>
        );
      }}
    />
  );
}
