"use strict";
/**
 * Minimal mirror of mira-server's WebSocket protocol.
 *
 * We only declare the frames the extension actually cares about — every
 * unknown frame is dropped silently on the client side, which keeps the
 * extension forward-compatible when the server adds new variants.
 *
 * Kept in sync with `crates/mira-server/src/protocol.rs`. If a field
 * name diverges, this file is the wrong one — the server owns the wire.
 */
Object.defineProperty(exports, "__esModule", { value: true });
//# sourceMappingURL=types.js.map