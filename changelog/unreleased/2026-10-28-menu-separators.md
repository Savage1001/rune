- **Separators in application and Dock menus.** New `MenuItem::separator()` puts a separator
  line among an app's custom items, placed with `.menu("File")` or `.dock()` like any item
  (`NSMenuItem.separatorItem` on macOS). Separators need no id, can't be fired and are skipped
  by id/title lookup. Windows and Linux still report `Unsupported` for native menus.
