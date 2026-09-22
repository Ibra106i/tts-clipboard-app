import { harness, type CommandArgs } from './harness'

export async function invoke<T = unknown>(
  cmd: string,
  args: CommandArgs = {},
): Promise<T> {
  return harness.invoke<T>(cmd, args)
}

export class Channel<T = unknown> {
  onmessage: ((message: T) => void) | undefined

  send(message: T): void {
    this.onmessage?.(message)
  }
}

export type InvokeArgs = CommandArgs
