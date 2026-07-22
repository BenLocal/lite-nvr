// Pure helpers for the detection overlay: a stable color palette and the
// frame-space -> screen-space mapping under `object-fit: contain`.

export const DETECT_PALETTE = [
  '#22d3ee', // cyan
  '#f472b6', // pink
  '#a3e635', // lime
  '#fbbf24', // amber
  '#c084fc', // violet
  '#fb7185', // rose
]

/** Stable color for a model by its index in the configured-model list. */
export function colorForIndex(i: number): string {
  const n = DETECT_PALETTE.length
  const index = ((i % n) + n) % n
  return DETECT_PALETTE[index]!
}

export interface ScreenRect {
  x: number
  y: number
  w: number
  h: number
}

/**
 * Map a box in frame-pixel space (`frameW × frameH`) to on-screen CSS pixels
 * within a `viewW × viewH` element whose content is displayed with
 * `object-fit: contain` (aspect preserved, letterboxed, centered).
 *
 * Example: box (100,100)-(200,200) in a 800×600 frame shown in a 400×400 view
 * → scale = min(400/800, 400/600) = 0.5; offsetY = (400 - 600*0.5)/2 = 50
 * → { x: 50, y: 100, w: 50, h: 50 }.
 */
export function frameToScreen(
  box: { x1: number; y1: number; x2: number; y2: number },
  frameW: number,
  frameH: number,
  viewW: number,
  viewH: number,
): ScreenRect {
  if (frameW <= 0 || frameH <= 0) {
    return { x: 0, y: 0, w: 0, h: 0 }
  }
  const scale = Math.min(viewW / frameW, viewH / frameH)
  const offsetX = (viewW - frameW * scale) / 2
  const offsetY = (viewH - frameH * scale) / 2
  return {
    x: box.x1 * scale + offsetX,
    y: box.y1 * scale + offsetY,
    w: (box.x2 - box.x1) * scale,
    h: (box.y2 - box.y1) * scale,
  }
}
