/** Render MCP text/images without showing transport envelopes or base64. */
export function toolPresentation(content) {
  const text = typeof content === 'string' ? content : '';
  let value;
  try {
    value = JSON.parse(text);
  } catch {
    return { text, images: [] };
  }
  if (!value || !Array.isArray(value.content)) return { text, images: [] };
  const parts = [];
  const images = [];
  for (const block of value.content) {
    if (block?.type === 'text' && typeof block.text === 'string') {
      parts.push(block.text);
    } else if (
      block?.type === 'image' && typeof block.data === 'string' &&
      /^image\/(png|jpeg|webp|gif)$/.test(block.mimeType)
    ) {
      images.push({ media_type: block.mimeType, data: block.data });
    } else if (block?.type === 'image') {
      parts.push('Image unavailable.');
    } else if (block != null) {
      parts.push(JSON.stringify(block, null, 2));
    }
  }
  if (!parts.length && !images.length && value.structuredContent != null) {
    parts.push(JSON.stringify(value.structuredContent, null, 2));
  }
  return { text: parts.join('\n\n'), images, isError: value.isError === true };
}
