/**
 * Pure display helpers shared by the overlay and the reader.
 *
 * Units: every length here is a count of *characters* (Unicode scalar values),
 * which is the unit the backend reports playback progress in.
 */

/**
 * Baseline narration speed used only to estimate a total duration before the
 * backend has reported one. Measured against the default SAPI voice; it is a
 * guess by design and is always superseded by real progress data.
 */
export const CHARS_PER_SECOND_AT_1X = 15

/** Format a millisecond duration as `M:SS` (or `H:MM:SS` past an hour). */
export function formatTime(ms: number): string {
  const safeMs = Number.isFinite(ms) && ms > 0 ? ms : 0
  const totalSeconds = Math.floor(safeMs / 1000)
  const seconds = totalSeconds % 60
  const minutes = Math.floor(totalSeconds / 60) % 60
  const hours = Math.floor(totalSeconds / 3600)
  const paddedSeconds = seconds.toString().padStart(2, '0')
  if (hours > 0) {
    return `${hours}:${minutes.toString().padStart(2, '0')}:${paddedSeconds}`
  }
  return `${minutes}:${paddedSeconds}`
}

/**
 * Estimate how long `charCount` characters take to narrate at `rate`.
 *
 * Playback rate is a multiplier (1 = normal, 2 = twice as fast), so speech
 * takes `1 / rate` as long. Progress reporting from the backend remains the
 * source of truth once playback starts.
 */
export function estimateDurationMs(
  charCount: number,
  rate = 1,
): number {
  if (!Number.isFinite(charCount) || charCount <= 0) {
    return 0
  }
  const safeRate = Number.isFinite(rate) && rate > 0 ? rate : 1
  return (charCount / (CHARS_PER_SECOND_AT_1X * safeRate)) * 1000
}

/** Clamp a value into `0..100` for progress bars; NaN becomes 0. */
export function clampPercent(value: number): number {
  if (!Number.isFinite(value)) {
    return 0
  }
  return Math.min(Math.max(value, 0), 100)
}

/** Percentage of a whole, guarding against a zero or invalid total. */
export function percentOf(part: number, whole: number): number {
  if (!Number.isFinite(part) || !Number.isFinite(whole) || whole <= 0) {
    return 0
  }
  return clampPercent((part / whole) * 100)
}

/**
 * A clamped percentage as a CSS-ready string, e.g. `"42.5%"`.
 * Used for progress bar widths so a bad value can never produce `NaN%`.
 */
export function formatPercent(value: number): string {
  return `${clampPercent(value)}%`
}

/** Shorten `text` for display without splitting a surrogate pair. */
export function previewText(text: string, maxChars: number): string {
  const characters = Array.from(text)
  if (characters.length <= maxChars) {
    return text
  }
  return `${characters.slice(0, maxChars).join('')}...`
}
