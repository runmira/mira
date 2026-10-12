/** Filled, friendly artwork in the same palette as the release-channel illustrations. */
export function ComputerIllustration({ className = 'size-10' }: { className?: string }) {
  return <svg aria-hidden="true" viewBox="0 0 48 48" className={className} fill="none">
    <path d="M9 10.5h29a3 3 0 0 1 3 3V32a3 3 0 0 1-3 3H9a3 3 0 0 1-3-3V13.5a3 3 0 0 1 3-3Z" fill="#c4b5fd" stroke="#8b5cf6" strokeWidth="1.3" />
    <path d="M10.5 14h26a1 1 0 0 1 1 1v14.5a1 1 0 0 1-1 1h-26a1 1 0 0 1-1-1V15a1 1 0 0 1 1-1Z" fill="#ede9fe" />
    <path d="M20 35h7l1.5 5h-10L20 35Z" fill="#a78bfa" />
    <path d="M16.5 40h14" stroke="#8b5cf6" strokeWidth="2.5" strokeLinecap="round" />
    <circle cx="18.5" cy="21" r="1.4" fill="#5b21b6" /><circle cx="28.5" cy="21" r="1.4" fill="#5b21b6" />
    <path d="M20 25q3.5 3.5 7 0" stroke="#5b21b6" strokeWidth="1.5" strokeLinecap="round" />
    <path d="M12 17h4" stroke="#fff" strokeWidth="2" strokeLinecap="round" />
    <path d="m40 4 1.5 4.5L46 10l-4.5 1.5L40 16l-1.5-4.5L34 10l4.5-1.5L40 4Z" fill="#fbbf24" stroke="#f59e0b" strokeWidth=".8" strokeLinejoin="round" />
    <circle cx="6" cy="39" r="1.5" fill="#c4b5fd" />
  </svg>;
}
