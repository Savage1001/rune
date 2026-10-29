- **`FrameEncoded::update_ms` and `FrameEncoded::encode_ms`.** A `Drawn` frame's report says how
  long the runtime took to build it and how long encoding took until submit, the numbers a
  compositor gets. Logging frame timing from `App::on_frame_encoded` no longer needs a
  compositor, which would turn off the window's skip of unchanged timer frames. Other outcomes
  report `None`.
