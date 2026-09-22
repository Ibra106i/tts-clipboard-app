/**
 * In-process stand-in for the Tauri IPC bridge.
 *
 * The real bridge only exists inside a webview, so every test that renders a
 * component which talks to the backend drives this harness instead. The mock
 * modules in this directory (`tauri-core.ts`, `tauri-event.ts`, ...) are aliased
 * onto the real `@tauri-apps/api/*` packages by `vite.config.ts`.
 */

export type CommandArgs = Record<string, unknown>
export type CommandHandler = (args: CommandArgs) => unknown
export type Listener = (event: TauriEventMessage) => void

export interface TauriEventMessage {
  event: string
  id: number
  payload: unknown
}

export interface InvokeCall {
  cmd: string
  args: CommandArgs
}

export interface WindowCall {
  method: string
  args: unknown[]
}

const STORAGE_MISSING = 'tauri harness: no handler registered for command'

class TauriHarness {
  readonly handlers = new Map<string, CommandHandler>()
  readonly listeners = new Map<string, Map<number, Listener>>()
  readonly calls: InvokeCall[] = []
  readonly windowCalls: WindowCall[] = []
  readonly dragDropListeners = new Set<Listener>()

  windowLabel = 'main'
  hidden = false
  focused = false

  private nextId = 1

  reset(): void {
    this.handlers.clear()
    this.listeners.clear()
    this.calls.length = 0
    this.windowCalls.length = 0
    this.dragDropListeners.clear()
    this.windowLabel = 'main'
    this.hidden = false
    this.focused = false
    this.nextId = 1
  }

  /** Register a command implementation, e.g. `on('library_get', () => [])`. */
  on(cmd: string, handler: CommandHandler): void {
    this.handlers.set(cmd, handler)
  }

  /** Register several commands at once. */
  onAll(handlers: Record<string, CommandHandler>): void {
    for (const [cmd, handler] of Object.entries(handlers)) {
      this.on(cmd, handler)
    }
  }

  async invoke<T = unknown>(cmd: string, args: CommandArgs = {}): Promise<T> {
    this.calls.push({ cmd, args })
    const handler = this.handlers.get(cmd)
    if (!handler) {
      throw new Error(`${STORAGE_MISSING} "${cmd}"`)
    }
    return (await handler(args)) as T
  }

  async listen<T = unknown>(
    event: string,
    handler: (message: { payload: T }) => void,
  ): Promise<() => void> {
    const id = this.nextId++
    const listeners = this.listeners.get(event) ?? new Map<number, Listener>()
    listeners.set(id, handler as Listener)
    this.listeners.set(event, listeners)
    return () => {
      listeners.delete(id)
    }
  }

  /** Number of live listeners for an event (regression tests for leaks). */
  listenerCount(event: string): number {
    return this.listeners.get(event)?.size ?? 0
  }

  /** Total live listeners across every event. */
  totalListenerCount(): number {
    let total = 0
    for (const listeners of this.listeners.values()) {
      total += listeners.size
    }
    return total
  }

  /** Number of drag/drop subscribers on the current webview. */
  get dragDropListenerCount(): number {
    return this.dragDropListeners.size
  }

  /** Simulate a backend -> frontend event. */
  emit<T = unknown>(event: string, payload: T): void {
    const listeners = this.listeners.get(event)
    if (!listeners) {
      return
    }
    for (const [id, listener] of [...listeners]) {
      listener({ event, id, payload })
    }
  }

  /** Simulate `tauri://drag-over` / `tauri://drag-drop` / `tauri://drag-leave`. */
  emitDragDrop(payload: unknown): void {
    const message: TauriEventMessage = {
      event: 'tauri://drag-drop',
      id: 0,
      payload,
    }
    for (const listener of [...this.dragDropListeners]) {
      listener(message)
    }
  }

  /** Calls recorded for a single command. */
  callsFor(cmd: string): InvokeCall[] {
    return this.calls.filter((call) => call.cmd === cmd)
  }

  windowCallsFor(method: string): WindowCall[] {
    return this.windowCalls.filter((call) => call.method === method)
  }
}

export const harness = new TauriHarness()

/** Structured error shape returned by backend commands. */
export interface BackendError {
  code: string
  message: string
  detail: string | null
}

/** Build the structured error payload the Rust backend serializes. */
export function backendError(
  code: string,
  message: string,
  detail: string | null = null,
): BackendError {
  return { code, message, detail }
}
