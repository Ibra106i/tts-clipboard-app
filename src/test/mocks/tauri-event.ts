import { harness } from './harness'

export type UnlistenFn = () => void

export interface Event<T> {
  event: string
  id: number
  payload: T
}

export async function listen<T = unknown>(
  event: string,
  handler: (message: Event<T>) => void,
): Promise<UnlistenFn> {
  return harness.listen<T>(event, handler as (message: { payload: T }) => void)
}

export async function once<T = unknown>(
  event: string,
  handler: (message: Event<T>) => void,
): Promise<UnlistenFn> {
  const unlisten = await listen<T>(event, (message) => {
    unlisten()
    handler(message)
  })
  return unlisten
}

export async function emit<T = unknown>(event: string, payload?: T): Promise<void> {
  harness.emit(event, payload)
}
