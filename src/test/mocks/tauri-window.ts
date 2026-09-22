import { harness, type Listener } from './harness'

export type UnlistenFn = () => void

export interface Window {
  label: string
  hide(): Promise<void>
  show(): Promise<void>
  setFocus(): Promise<void>
  close(): Promise<void>
  onDragDropEvent(handler: (event: { payload: unknown }) => void): Promise<UnlistenFn>
}

function createWindow(): Window {
  return {
    get label() {
      return harness.windowLabel
    },
    async hide() {
      harness.windowCalls.push({ method: 'hide', args: [] })
      harness.hidden = true
    },
    async show() {
      harness.windowCalls.push({ method: 'show', args: [] })
      harness.hidden = false
    },
    async setFocus() {
      harness.windowCalls.push({ method: 'setFocus', args: [] })
      harness.focused = true
    },
    async close() {
      harness.windowCalls.push({ method: 'close', args: [] })
    },
    async onDragDropEvent(handler) {
      const listener = handler as Listener
      harness.dragDropListeners.add(listener)
      return () => {
        harness.dragDropListeners.delete(listener)
      }
    },
  }
}

export function getCurrentWindow(): Window {
  return createWindow()
}
