# Third-party source adaptations

## ZCode reasoning previews and activity gradient

Mira's thinking preview behavior and activity-gradient CSS are adapted from
[zai-org/ZCode](https://github.com/zai-org/ZCode), licensed under Apache-2.0.
Relevant sources: `packages/ui/src/components/ai-elements/reasoning.tsx`,
`packages/ui/src/ToolCallBlocks/QueuedSummaryContent.tsx` and
`packages/ui/src/styles.css`. The reasoning component identifies its original
source as vercel/ai-elements, Copyright 2023 Vercel, Inc., Apache-2.0, modified
by ZCode.

Mira modifications: scoped disclosure integration, CSS-only latest-line entrance,
Mira theme tokens and reduced-motion handling, contained scrolling, and no new
runtime dependencies. The Apache-2.0 license is included in the repository's
`LICENSE` file. This notice does not imply endorsement by ZCode or Vercel.
