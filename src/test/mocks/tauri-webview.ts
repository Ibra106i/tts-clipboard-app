import { harness, type Listener } from './harness'

export type UnlistenFn = () => void

export interface Webview {
  onDragDropEvent(handler: (event: { payload: unknown }) => void): Promise<UnlistenFn>
}

export function getCurrentWebview(): Webview {
  return {
    async onDragDropEvent(handler) {
      const listener = handler as Listener
      harness.dragDropListeners.add(listener)
      return () => {
        harness.dragDropListeners.delete(listener)
      }
    },
  }
}
