- **A window with placed traffic lights no longer crashes entering full screen on macOS.** Going
  full screen, AppKit observes the titlebar container and puts its own KVO subclass above the one
  `Chrome::traffic_lights` gave it, so the layout hook called AppKit's `-layout` through its own
  class again and overflowed the stack. The hook now finds Rune's subclass in the chain and calls
  the class above it, and the buttons still return to their spot on leaving full screen.
