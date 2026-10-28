- **Icons honour SVG opacity.** `opacity`, `fill-opacity` and `stroke-opacity` on an icon's
  elements now scale that element's coverage, so one `Icon` can hold a graded shape (a faint
  track under an arc, dots fading round a spinner). Before, they were ignored and drew solid.
