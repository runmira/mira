/**
 * Starts `mira serve` when nothing answers at `mira.baseUrl`.
 *
 * Only for loopback URLs: a remote URL means the user runs the server
 * somewhere else on purpose. The process lives as long as the window;
 * its output goes to the "Mira" output channel.
 */

import { spawn, type ChildProcess } from 'child_process';
import * as vscode from 'vscode';

const LOOPBACK = new Set(['127.0.0.1', 'localhost', '[::1]', '::1']);
const START_TIMEOUT_MS = 20_000;

export class ServerLauncher implements vscode.Disposable {
  private child: ChildProcess | null = null;
  private starting: Promise<boolean> | null = null;

  constructor(private readonly output: vscode.OutputChannel) {}

  /** True once something answers `/api/health` at `baseUrl`. */
  async isUp(baseUrl: string): Promise<boolean> {
    try {
      const res = await fetch(new URL('/api/health', baseUrl), {
        signal: AbortSignal.timeout(1500),
      });
      return res.ok;
    } catch {
      return false;
    }
  }

  /** Make sure a server answers at `baseUrl`, starting one if allowed.
   *  Resolves false when it can't (auto-start off, remote URL, missing
   *  binary, or it never came up). */
  async ensure(baseUrl: string): Promise<boolean> {
    if (await this.isUp(baseUrl)) return true;
    const cfg = vscode.workspace.getConfiguration('mira');
    if (!cfg.get<boolean>('autoStart', true)) return false;
    const url = new URL(baseUrl);
    if (!LOOPBACK.has(url.hostname)) return false;
    this.starting ??= this.start(url, cfg.get<string>('path', 'mira') || 'mira').finally(() => {
      this.starting = null;
    });
    return this.starting;
  }

  private async start(url: URL, binary: string): Promise<boolean> {
    const port = url.port || (url.protocol === 'https:' ? '443' : '80');
    const cwd = vscode.workspace.workspaceFolders?.[0]?.uri.fsPath;
    this.output.appendLine(`Starting ${binary} serve --port ${port}${cwd ? ` in ${cwd}` : ''}`);
    let failed: string | null = null;
    const child = spawn(binary, ['serve', '--port', port, '--host', '127.0.0.1'], {
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
      if (this.child === child) this.child = null;
    });
    this.child = child;

    const deadline = Date.now() + START_TIMEOUT_MS;
    while (Date.now() < deadline) {
      if (failed || child.exitCode !== null) return false;
      if (await this.isUp(url.toString())) return true;
      await new Promise((r) => setTimeout(r, 400));
    }
    return false;
  }

  /** True when this window started the server. */
  get owned(): boolean {
    return this.child !== null;
  }

  dispose(): void {
    this.child?.kill();
    this.child = null;
  }
}
