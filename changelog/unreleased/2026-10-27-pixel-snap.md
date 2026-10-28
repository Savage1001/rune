- **Fills, borders and clips land on whole device pixels.** A box laid out at a fractional
  position (a 1.1 pt gap at 2×) drew soft, half-covered edge rows. Each painted rect now rounds
  its four edges to the device grid at paint time, so edges are hard, boxes that share an edge
  meet with no seam, a 0.5 pt border at 2× is one crisp pixel, and text keeps the same offset
  from its fill wherever the box lands. Layout, hit testing, `Cx::layout_size` and
  `Runtime::elements` keep the exact fractional rects. Motion stays smooth: a moving element's
  correction eases from the one it drew with toward the rounding of where it comes to rest, in
  step with the distance left, so it glides between pixels with no jump at the start or the
  landing; a scroll snaps its content as one
  translation, and a scale fades the snap in over its last pixel. `El::pixel_snap(false)` turns
  it off for a subtree an app moves itself at sub-pixel speeds.
