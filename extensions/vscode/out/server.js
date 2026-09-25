"use strict";
/**
 * Starts `mira serve` when nothing answers at `mira.baseUrl`.
 *
 * Only for loopback URLs: a remote URL means the user runs the server
 * somewhere else on purpose. The process lives as long as the window;
 * its output goes to the "Mira" output channel.
 */
var __createBinding = (this && this.__createBinding) || (Object.create ? (function(o, m, k, k2) {
    if (k2 === undefined) k2 = k;
    var desc = Object.getOwnPropertyDescriptor(m, k);
    if (!desc || ("get" in desc ? !m.__esModule : desc.writable || desc.configurable)) {
      desc = { enumerable: true, get: function() { return m[k]; } };
    }
    Object.defineProperty(o, k2, desc);
}) : (function(o, m, k, k2) {
    if (k2 === undefined) k2 = k;
    o[k2] = m[k];
}));
var __setModuleDefault = (this && this.__setModuleDefault) || (Object.create ? (function(o, v) {
    Object.defineProperty(o, "default", { enumerable: true, value: v });
}) : function(o, v) {
    o["default"] = v;
});
var __importStar = (this && this.__importStar) || (function () {
    var ownKeys = function(o) {
        ownKeys = Object.getOwnPropertyNames || function (o) {
            var ar = [];
            for (var k in o) if (Object.prototype.hasOwnProperty.call(o, k)) ar[ar.length] = k;
            return ar;
        };
        return ownKeys(o);
    };
    return function (mod) {
        if (mod && mod.__esModule) return mod;
        var result = {};
        if (mod != null) for (var k = ownKeys(mod), i = 0; i < k.length; i++) if (k[i] !== "default") __createBinding(result, mod, k[i]);
        __setModuleDefault(result, mod);
        return result;
    };
})();
Object.defineProperty(exports, "__esModule", { value: true });
exports.ServerLauncher = void 0;
const child_process_1 = require("child_process");
const vscode = __importStar(require("vscode"));
const LOOPBACK = new Set(['127.0.0.1', 'localhost', '[::1]', '::1']);
const START_TIMEOUT_MS = 20_000;
class ServerLauncher {
    output;
    child = null;
    starting = null;
    constructor(output) {
        this.output = output;
    }
    /** True once something answers `/api/health` at `baseUrl`. */
    async isUp(baseUrl) {
        try {
            const res = await fetch(new URL('/api/health', baseUrl), {
                signal: AbortSignal.timeout(1500),
            });
            return res.ok;
        }
        catch {
            return false;
        }
    }
    /** Make sure a server answers at `baseUrl`, starting one if allowed.
     *  Resolves false when it can't (auto-start off, remote URL, missing
     *  binary, or it never came up). */
    async ensure(baseUrl) {
        if (await this.isUp(baseUrl))
            return true;
        const cfg = vscode.workspace.getConfiguration('mira');
        if (!cfg.get('autoStart', true))
            return false;
        const url = new URL(baseUrl);
        if (!LOOPBACK.has(url.hostname))
            return false;
        this.starting ??= this.start(url, cfg.get('path', 'mira') || 'mira').finally(() => {
            this.starting = null;
        });
        return this.starting;
    }
    async start(url, binary) {
        const port = url.port || (url.protocol === 'https:' ? '443' : '80');
        const cwd = vscode.workspace.workspaceFolders?.[0]?.uri.fsPath;
        this.output.appendLine(`Starting ${binary} serve --port ${port}${cwd ? ` in ${cwd}` : ''}`);
        let failed = null;
        const child = (0, child_process_1.spawn)(binary, ['serve', '--port', port, '--host', '127.0.0.1'], {
            cwd,
            env: process.env,
            stdio: ['ignore', 'pipe', 'pipe'],
        });
        child.stdout?.on('data', (d) => this.output.append(d.toString()));
        child.stderr?.on('data', (d) => this.output.append(d.toString()));
        child.on('error', (e) => {
            failed = e.message;
            this.output.appendLine(`Couldn't start ${binary}: ${e.message}`);
        });
        child.on('exit', (code) => {
            this.output.appendLine(`mira serve exited (${code ?? 'signal'})`);
            if (this.child === child)
                this.child = null;
        });
        this.child = child;
        const deadline = Date.now() + START_TIMEOUT_MS;
        while (Date.now() < deadline) {
            if (failed || child.exitCode !== null)
                return false;
            if (await this.isUp(url.toString()))
                return true;
            await new Promise((r) => setTimeout(r, 400));
        }
        return false;
    }
    /** True when this window started the server. */
    get owned() {
        return this.child !== null;
    }
    dispose() {
        this.child?.kill();
        this.child = null;
    }
}
exports.ServerLauncher = ServerLauncher;
//# sourceMappingURL=server.js.map