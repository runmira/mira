/** Development-only fixture: no model calls or real filesystem mutations. */
import { useEffect, useState } from 'react';
import { createRoot } from 'react-dom/client';
import { GuidedWalkthrough } from './components/GuidedWalkthrough';
import './styles.css';
import { setThemePref } from './lib/theme';
const patches = [
  'diff --git a/src/save.ts b/src/save.ts\n--- a/src/save.ts\n+++ b/src/save.ts\n@@ -1,4 +1,5 @@\n export async function save(path: string) {\n-  writeFile(path);\n+  await writeFile(path);\n+  await syncDirectory(path);\n   return path;\n }\n',
  'diff --git a/src/save.test.ts b/src/save.test.ts\nnew file mode 100644\n--- /dev/null\n+++ b/src/save.test.ts\n@@ -0,0 +1,3 @@\n+test("save waits for durable writes", async () => {\n+  expect(await save("draft")).toBe("draft");\n+});\n',
  'diff --git a/README.md b/README.md\n--- a/README.md\n+++ b/README.md\n@@ -1 +1 @@\n-# Saving\n+# Durable saving\n',
];
const originalFetch = window.fetch.bind(window);
window.fetch = async (input, init) => {
  const url = typeof input === 'string' ? input : input instanceof URL ? input.pathname : input.url;
  if (!url.startsWith('/api/walkthrough')) return originalFetch(input, init);
  const value = url.endsWith('/question') ? { answer: 'Awaiting both operations means save resolves after the file and its containing directory have been synced. The diff does not show how syncDirectory handles errors, so verify that implementation.' } : url.endsWith('/revert') ? { ok: true } : {
    files: patches, stops: [
      { file_index: 0, category: 'core', summary: 'Wait for the file write and directory sync before reporting success. This makes the save completion reflect durable storage rather than scheduling the write.', risks: ['New I/O: syncDirectory can fail after the file has been written (line 3). Verify error recovery.'] },
      { file_index: 1, category: 'tests', summary: 'Add coverage for the asynchronous save result. Check that this also verifies the write and sync have finished.', risks: [] },
      { file_index: 2, category: 'incidental', summary: 'Rename the documentation heading to match the durable-save behavior.', risks: [] },
    ],
  };
  await new Promise(resolve => setTimeout(resolve, 200));
  return new Response(JSON.stringify(value), { headers: { 'content-type': 'application/json' } });
};
function Preview() {
  const [dark, setDark] = useState(false);
  useEffect(() => setThemePref(dark ? 'dark' : 'light'), [dark]);
  return <main className="flex h-screen flex-col bg-background text-foreground" data-theme={dark ? 'dark' : 'light'}>
    <header className="flex items-center gap-4 border-b border-border px-4 py-2 text-sm"><h1>Guided review · fixture preview</h1><button onClick={() => setDark(!dark)}>Switch theme</button></header>
    <div className="min-h-0 flex-1"><GuidedWalkthrough onChanged={() => {}} onClose={() => {}} /></div>
  </main>;
}
createRoot(document.getElementById('root')!).render(<Preview />);
