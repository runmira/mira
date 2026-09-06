import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { IconContext } from '@phosphor-icons/react';
import App from './App';
import './styles.css';

// Global icon defaults: duotone weight for that two-tone dynamic feel, and
// `size="1em"` so tailwind size-* classes (which set width/height in CSS)
// still control the icon size — they override the SVG's own width attr.
createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <IconContext.Provider value={{ weight: 'duotone', size: '1em', mirrored: false }}>
      <App />
    </IconContext.Provider>
  </StrictMode>,
);
