- **A key press on a resting macOS window draws at once.** A window that had stopped its
  display link started it on input and waited for its first tick before drawing, so the first
  frame after a key, click or paste ran its handler about 12 ms late (median, `--profile perf`).
  When the last frame reached the display at least a period and a half ago, the window now draws
  in the same run-loop turn, and its handlers run about 0.3 ms after the event (median). Frames while
  something moves still wait for the display's tick, and a Low Power cap still spaces them.
  `FrameEncoded::presents_at()` is `None` for such a frame.
