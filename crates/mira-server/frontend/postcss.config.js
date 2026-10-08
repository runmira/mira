import tailwindcss from 'tailwindcss';
import autoprefixer from 'autoprefixer';

// Scale absolute type sizes without changing spacing, icons, or webview zoom.
// Relative em sizes inherit the reduction and must not be scaled twice.
const typographyScale = {
  postcssPlugin: 'mira-typography-scale',
  OnceExit(root) {
    root.walkDecls('font-size', declaration => {
      if (/^\d*\.?\d+(px|rem)$/.test(declaration.value.trim())) {
        declaration.value = `calc(${declaration.value} * var(--mira-font-scale, 0.86))`;
      }
    });
  },
};
export default { plugins: [tailwindcss(), typographyScale, autoprefixer()] };
