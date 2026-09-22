/**
 * Normalising the many shapes a failure can arrive in.
 *
 * Backend commands reject with `{ code, message, detail }` (see
 * `src-tauri/src/error.rs`). During the migration to that shape some paths still
 * reject with plain strings, and local code can throw `Error` instances, so
 * everything funnels through one function instead of being stringified ad hoc.
 */

export interface BackendError extends Record<string, unknown> {
  code: string
  message: string
  detail: string | null
}

/** Machine codes the frontend branches on. Keep in sync with `AppError::code`. */
export const ERROR_CODES = {
  notFound: 'not_found',
  invalidInput: 'invalid_input',
  unsupportedPlatform: 'unsupported_platform',
  importFailed: 'import_failed',
  documentParseFailed: 'document_parse_failed',
  storageFailed: 'storage_failed',
  playbackFailed: 'playback_failed',
  busy: 'busy',
  internal: 'internal_error',
} as const

export type ErrorCode = (typeof ERROR_CODES)[keyof typeof ERROR_CODES]

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null
}

/** True when `value` looks like a serialized backend error. */
export function isBackendError(value: unknown): value is BackendError {
  return (
    isRecord(value) &&
    typeof value['code'] === 'string' &&
    typeof value['message'] === 'string'
  )
}

/** Compare any thrown value against a backend error code. */
export function isErrorCode(value: unknown, code: ErrorCode): boolean {
  if (isBackendError(value)) {
    return value.code === code
  }
  // Legacy paths rejected with the code as a bare string.
  return value === code
}

/**
 * Human-readable text for any thrown value. Never returns `[object Object]`
 * and never throws, so it is safe to call from a toast or an error boundary.
 */
export function describeError(value: unknown): string {
  if (isBackendError(value)) {
    return value.message
  }
  if (typeof value === 'string') {
    return value
  }
  if (value instanceof Error) {
    return value.message
  }
  if (isRecord(value)) {
    const message = value['message']
    if (typeof message === 'string') {
      return message
    }
    try {
      return JSON.stringify(value)
    } catch {
      return 'Unknown error'
    }
  }
  if (value === null || value === undefined) {
    return 'Unknown error'
  }
  return String(value)
}

/** Diagnostic detail for logs, when the backend supplied any. */
export function errorDetail(value: unknown): string | null {
  if (isBackendError(value)) {
    return value.detail
  }
  if (value instanceof Error) {
    return value.stack ?? value.message
  }
  return null
}

/** The machine code of a thrown value, if it has one. */
export function errorCode(value: unknown): ErrorCode | string | null {
  if (isBackendError(value)) {
    return value.code
  }
  if (typeof value === 'string' && value in ERROR_CODES) {
    return value
  }
  return null
}
