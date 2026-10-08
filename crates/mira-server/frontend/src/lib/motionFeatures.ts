// framer-motion's animation features, loaded after first paint via
// <LazyMotion> in main.tsx (issue #72). domMax rather than domAnimation
// because the composer uses a `layout` animation.
export { domMax as default } from 'framer-motion';
