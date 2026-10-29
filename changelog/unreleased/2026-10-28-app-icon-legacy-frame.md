- **macOS app icons at small sizes are the app's real icon.** `app_icon_rgba` (and
  `ImageCache::app_icon`) now draws the icon at 256 pixels or more and scales it down. On macOS 26
  and later, asking for a small size directly could return the system's "legacy" rendition, a
  shrunken tile inside a grey rounded frame.
