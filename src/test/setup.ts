import '@testing-library/jest-dom/vitest'
import { cleanup } from '@testing-library/react'
import { afterEach, beforeEach, vi } from 'vitest'
import { harness } from './mocks/harness'

beforeEach(() => {
  harness.reset()
})

afterEach(() => {
  cleanup()
  vi.useRealTimers()
})

// jsdom does not implement matchMedia, which the dark UI shell queries.
if (!window.matchMedia) {
  window.matchMedia = ((query: string) => ({
    matches: false,
    media: query,
    onchange: null,
    addListener: () => {},
    removeListener: () => {},
    addEventListener: () => {},
    removeEventListener: () => {},
    dispatchEvent: () => false,
  })) as unknown as typeof window.matchMedia
}
