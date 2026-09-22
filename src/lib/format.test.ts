import { describe, expect, it } from 'vitest'
import {
  CHARS_PER_SECOND_AT_1X,
  clampPercent,
  estimateDurationMs,
  formatTime,
  percentOf,
  previewText,
} from './format'

describe('formatTime', () => {
  it('formats sub-minute and multi-minute durations', () => {
    expect(formatTime(0)).toBe('0:00')
    expect(formatTime(1_000)).toBe('0:01')
    expect(formatTime(59_000)).toBe('0:59')
    expect(formatTime(60_000)).toBe('1:00')
    expect(formatTime(605_000)).toBe('10:05')
  })

  it('adds an hour component only when needed', () => {
    expect(formatTime(3_600_000)).toBe('1:00:00')
    expect(formatTime(3_661_000)).toBe('1:01:01')
  })

  it('treats invalid and negative input as zero instead of rendering NaN', () => {
    expect(formatTime(Number.NaN)).toBe('0:00')
    expect(formatTime(-5_000)).toBe('0:00')
    expect(formatTime(Number.POSITIVE_INFINITY)).toBe('0:00')
  })
})

describe('estimateDurationMs', () => {
  it('uses the baseline narration speed at rate 1', () => {
    expect(estimateDurationMs(CHARS_PER_SECOND_AT_1X)).toBe(1_000)
    expect(estimateDurationMs(CHARS_PER_SECOND_AT_1X * 60)).toBe(60_000)
  })

  it('accounts for playback rate', () => {
    expect(estimateDurationMs(CHARS_PER_SECOND_AT_1X, 2)).toBe(500)
    expect(estimateDurationMs(CHARS_PER_SECOND_AT_1X * 60, 1.5)).toBe(40_000)
  })

  it('returns zero for empty or invalid input', () => {
    expect(estimateDurationMs(0)).toBe(0)
    expect(estimateDurationMs(-10)).toBe(0)
    expect(estimateDurationMs(Number.NaN)).toBe(0)
  })

  it('falls back to rate 1 when given an invalid rate', () => {
    expect(estimateDurationMs(CHARS_PER_SECOND_AT_1X, 0)).toBe(1_000)
    expect(estimateDurationMs(CHARS_PER_SECOND_AT_1X, Number.NaN)).toBe(1_000)
  })
})

describe('clampPercent / percentOf', () => {
  it('clamps to 0..100', () => {
    expect(clampPercent(-1)).toBe(0)
    expect(clampPercent(50)).toBe(50)
    expect(clampPercent(150)).toBe(100)
    expect(clampPercent(Number.NaN)).toBe(0)
  })

  it('never divides by a zero or invalid total', () => {
    expect(percentOf(5, 0)).toBe(0)
    expect(percentOf(5, Number.NaN)).toBe(0)
    expect(percentOf(5, 10)).toBe(50)
  })
})

describe('previewText', () => {
  it('leaves short text alone', () => {
    expect(previewText('hello', 10)).toBe('hello')
  })

  it('truncates with an ellipsis', () => {
    expect(previewText('abcdefghij', 4)).toBe('abcd...')
  })

  it('never splits an astral character (emoji stay whole)', () => {
    const result = previewText('👍👍👍', 2)
    expect(result).toBe('👍👍...')
    expect(Array.from(result).length).toBe(5)
  })
})
