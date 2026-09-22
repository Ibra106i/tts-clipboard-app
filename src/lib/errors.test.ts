import { describe, expect, it } from 'vitest'
import {
  describeError,
  errorCode,
  errorDetail,
  isBackendError,
  isErrorCode,
} from './errors'
import { backendError } from '../test/mocks/harness'

describe('isBackendError', () => {
  it('recognises the structured shape', () => {
    expect(isBackendError(backendError('busy', 'Finish current playback first'))).toBe(true)
  })

  it('rejects strings, null and objects without a code', () => {
    expect(isBackendError('busy')).toBe(false)
    expect(isBackendError(null)).toBe(false)
    expect(isBackendError({ message: 'no code' })).toBe(false)
    expect(isBackendError(new Error('boom'))).toBe(false)
  })
})

describe('describeError', () => {
  it('prefers the backend user message', () => {
    expect(
      describeError(backendError('import_failed', 'Could not import book.epub: too large', 'io')),
    ).toBe('Could not import book.epub: too large')
  })

  it('handles legacy string rejections', () => {
    expect(describeError('busy')).toBe('busy')
  })

  it('handles Error instances and plain objects with a message', () => {
    expect(describeError(new Error('network down'))).toBe('network down')
    expect(describeError({ message: 'object message' })).toBe('object message')
  })

  it('never renders [object Object] and never throws', () => {
    const circular: Record<string, unknown> = {}
    circular['self'] = circular

    expect(describeError(circular)).toBe('Unknown error')
    expect(describeError({})).toBe('{}')
    expect(describeError(null)).toBe('Unknown error')
    expect(describeError(undefined)).toBe('Unknown error')
    expect(describeError(42)).toBe('42')
  })
})

describe('isErrorCode', () => {
  it('matches structured errors by code', () => {
    expect(isErrorCode(backendError('busy', 'x'), 'busy')).toBe(true)
    expect(isErrorCode(backendError('busy', 'x'), 'storage_failed')).toBe(false)
  })

  it('still matches the legacy bare-string form', () => {
    expect(isErrorCode('busy', 'busy')).toBe(true)
  })

  it('does not match unrelated values', () => {
    expect(isErrorCode(new Error('busy'), 'busy')).toBe(false)
    expect(isErrorCode(null, 'busy')).toBe(false)
  })
})

describe('errorDetail / errorCode', () => {
  it('exposes the diagnostic detail for logging', () => {
    expect(errorDetail(backendError('storage_failed', 'x', 'disk full'))).toBe('disk full')
    expect(errorDetail(new Error('boom'))).toBeTypeOf('string')
    expect(errorDetail('plain')).toBeNull()
  })

  it('exposes the machine code', () => {
    expect(errorCode(backendError('not_found', 'x'))).toBe('not_found')
    expect(errorCode('busy')).toBe('busy')
    expect(errorCode(new Error('boom'))).toBeNull()
  })
})
