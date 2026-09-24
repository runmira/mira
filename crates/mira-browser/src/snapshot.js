// Page snapshot for the `browser` tool.
//
// Evaluated as an expression: `(<this file>)(maxElements, maxText)`.
// Tags every interactive element in the viewport-reachable DOM with a
// stable `data-mira-ref` (e1, e2, ...) the model can target, and returns
// a compact text rendering plus the visible text.
(function (maxElements, maxText) {
  const SEL = [
    'a[href]', 'button', 'input:not([type=hidden])', 'select', 'textarea',
    'summary', '[role=button]', '[role=link]', '[role=checkbox]', '[role=radio]',
    '[role=tab]', '[role=menuitem]', '[role=option]', '[role=switch]',
    '[role=combobox]', '[role=textbox]', '[contenteditable=""]', '[contenteditable=true]',
  ].join(',');

  const visible = (el) => {
    const r = el.getBoundingClientRect();
    if (r.width < 1 || r.height < 1) return false;
    const s = getComputedStyle(el);
    return s.visibility !== 'hidden' && s.display !== 'none' && Number(s.opacity) > 0;
  };
  const clean = (s, n) => {
    s = (s || '').replace(/\s+/g, ' ').trim();
    return s.length > n ? s.slice(0, n - 1) + '…' : s;
  };
  const labelOf = (el) => {
    const aria = el.getAttribute('aria-label');
    if (aria) return aria;
    const by = el.getAttribute('aria-labelledby');
    if (by) {
      const t = by.split(/\s+/).map((id) => document.getElementById(id)?.innerText || '').join(' ');
      if (t.trim()) return t;
    }
    if (el.labels && el.labels.length) return el.labels[0].innerText;
    return el.innerText || el.value || el.getAttribute('placeholder') ||
      el.getAttribute('title') || el.getAttribute('alt') || el.getAttribute('name') || '';
  };
  const roleOf = (el) => {
    const r = el.getAttribute('role');
    if (r) return r;
    const t = el.tagName.toLowerCase();
    if (t === 'a') return 'link';
    if (t === 'input') {
      const ty = (el.getAttribute('type') || 'text').toLowerCase();
      if (['checkbox', 'radio', 'button', 'submit', 'reset', 'range', 'file'].includes(ty)) {
        return ty === 'submit' || ty === 'reset' ? 'button' : ty;
      }
      return 'textbox';
    }
    if (t === 'textarea' || el.isContentEditable) return 'textbox';
    if (t === 'select') return 'combobox';
    return t;
  };

  window.__miraRefSeq = window.__miraRefSeq || 0;
  const lines = [];
  let count = 0;
  let offscreen = 0;
  for (const el of document.querySelectorAll(SEL)) {
    if (!visible(el)) continue;
    if (count >= maxElements) { offscreen++; continue; }
    let ref = el.getAttribute('data-mira-ref');
    if (!ref) {
      ref = 'e' + (++window.__miraRefSeq);
      el.setAttribute('data-mira-ref', ref);
    }
    const role = roleOf(el);
    let line = `[${ref}] ${role} "${clean(labelOf(el), 80)}"`;
    if (role === 'link') {
      const href = el.getAttribute('href') || '';
      if (href && !href.startsWith('javascript:')) line += ` -> ${clean(href, 100)}`;
    }
    if (role === 'textbox' || role === 'combobox') {
      if (el.value) line += ` value="${clean(el.value, 60)}"`;
    }
    if ('checked' in el && (role === 'checkbox' || role === 'radio' || role === 'switch')) {
      line += el.checked ? ' [checked]' : ' [unchecked]';
    }
    if (el.disabled) line += ' [disabled]';
    const r = el.getBoundingClientRect();
    if (r.bottom < 0 || r.top > innerHeight) line += ' (offscreen)';
    lines.push(line);
    count++;
  }

  let text = document.body ? document.body.innerText || '' : '';
  text = text.replace(/\n{3,}/g, '\n\n').trim();
  const truncated = text.length > maxText;
  if (truncated) text = text.slice(0, maxText);

  return JSON.stringify({
    title: document.title,
    url: location.href,
    viewport: [innerWidth, innerHeight],
    scroll: [Math.round(scrollY), Math.max(0, document.documentElement.scrollHeight - innerHeight)],
    elements: lines,
    omitted: offscreen,
    text,
    textTruncated: truncated,
  });
})
