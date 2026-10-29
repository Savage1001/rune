- **`Tx::scroll_into_view` leaves a row taller than the viewport where it fills it.** Revealing
  such a row from above lands its end at the viewport's bottom, as before, but a virtual row's
  re-placement after its rows measure (or a follow frame while the list grows) no longer flips it
  to its start. Before, where it ended up depended on how many placements ran, so one extra
  trailing row could move it.
