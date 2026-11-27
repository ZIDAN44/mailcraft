# How the interface looks

The rules every pane, form and dialog follows, so MailCraft reads as one app. They come from
Windows' own design (Fluent 2), with Apple's and GNOME's guidelines where those differ, and from
an audit of every screen in 0.43 that found each of these decided many times over, differently.
The values live in `crates/ui-egui/src/look/sizes.rs`; the helpers named here are in
`look/widgets.rs`, `look/widgets/` and `dialogs/frame.rs`. Where a test guards a rule, it's named.

## Sizes

- **Type:** four sizes, `sizes::CAPTION` 12, `BODY` 14, `SUBTITLE` 20, `TITLE` 28. Semibold, never
  Bold, for what stands out. No size written as a number
  (`sizes::tests::text_sizes_come_from_the_ramp`); something that isn't text (an emoji, a spacer)
  is a named constant that says why.
- **Controls:** a dialog's buttons, text boxes and drop-downs are `CONTROL_H` 32 tall, the panes'
  and menus' `PANE_CONTROL_H` 26. A dialog's button is at least `BUTTON_MIN_W` 80 wide. A text box's
  words sit in its middle, top to bottom.
- **Corners:** `CONTROL_RADIUS` 4 on controls, rows and fills; `SURFACE_RADIUS` 8 on dialogs, menus,
  cards and boxes; circles for avatars; pills keep round ends.
- **Spacing,** on a 4-point grid: `GAP_S` 8 between a label and its field, `GAP_M` 16 between
  fields, `GAP_L` 24 between groups and as a dialog's margin.
- **Dialog widths:** `DIALOG_S` 360 (a question, one field), `DIALOG_M` 440 (a form), `DIALOG_L` 560
  (a list, an editor); something to read is at most `VIEWER_MAX` 720.
- **Lines:** one line between panes, none round a list or the ribbon's panel beside it; a scroll
  bar at rest is quiet (`theme::tests::a_scroll_bar_at_rest_is_quiet`).

## Colour

- The app colour (the accent) is for the one main button of a surface, the selection, the unread
  bar and links (`ui.link`, which the theme draws in it). Icons are in the text's colour, dim at
  rest.
- A destructive button (delete for good, empty, remove, replace) is red, `ButtonKind::Danger`.
- A disabled button looks unavailable (the chrome colour with a border), never like a dark one,
  and what blocks it is said under the field that does.

## Buttons and commands

- Three kinds, plus red: the accent button (one per surface: Send, Save, Connect), tool buttons
  (flat, a fill under the pointer) in toolbars, the ribbon and the reading pane, and outline
  buttons for a dialog's others.
- A dialog's buttons go through `button_row`, in the system's order: on Windows the main button
  first and **Cancel** last; on a Mac and Linux **Cancel** first. Enter presses the main button
  unless it's destructive; Esc is Cancel.
- Buttons say what they do ("Use this address", "Not now", "Delete folder"), never OK, Yes or No.
- A command is on a surface once: the calendar's views over the calendar, New on the ribbon,
  Reply in the reading pane, the list's filters over the list.
- Toolbars are one row: what doesn't fit goes into ⋯ (`fitted_toolbar`), labels drop before
  buttons do. A button with a menu has a drawn arrow (`menu_tool_button`); a split button's arrow
  is against it (`split_arrow`). An icon alone has a tooltip.

## Dialogs and forms

- Through `dialogs/frame.rs`: a 20-point Semibold title, the body scrolling while the buttons stay
  in sight, never taller than the window. A form isn't closed by a click beside it.
- A field's label is over it (`label_over`), never only a placeholder; fields fill the width they
  have, short ones (a port, a year) are as wide as what they hold. An error goes under its field
  (`field_error`), as a sentence that says what to do.
- A one-line text box is a `widgets::line_edit`, its words in its middle
  (`form_fields::tests::one_line_boxes_are_made_here`); a multi-line one keeps them at its top.
- A checkbox whose words may wrap uses `checkbox_wrapped`: the box beside the first line. A
  password has an eye to show it (`password_field`); a date has its month to pick from
  (`date_field`).
- **Ask or undo:** what can be undone happens at once, with Undo in the notice
  (`said_with_undo`). Only what can't be undone asks first, in a red button that isn't the default.
- Nothing found or nothing yet is said plainly, with what to do about it ("No rules yet" and New
  rule), never a blank.

## Words

- **Sentence case:** a label's first word alone has a capital ("Save as", "Current mailbox").
  Names keep theirs: products, the special folders (Deleted Items, Junk Email), and features
  (Quick Steps, Search Folders, Scheduling Assistant)
  (`i18n::tests::labels_are_in_sentence_case`).
- No full stop after a label, a button or a one-line description; a description is one line.
- Every string goes through `tl!`/`tf!` and is in every catalog. Names MailCraft gives things are
  shown translated: built-in categories (`i18n::category`), a calendar called "Calendar"
  (`i18n::calendar`), numbers as the language writes them (`i18n::number`).
- **Dates** in the window are written as the message list writes them: `dates::day_short` for a
  day and `dates::when_text` for a moment ("Sat Nov 28 03:41" this year, the whole date otherwise,
  in the order the Dates option says). `dates::full_time` is for text that leaves the window (a
  reply's Sent line, certificates, the CLI).

## Icons

- Drawn in code in `look/icons.rs` on a 16-unit grid, MailCraft's own.
- One glyph, one meaning: a new meaning gets a new glyph, never one that already stands for
  something else (`look::icons::tests::a_glyph_stands_for_one_thing`).

## Panes and lists

- A list row has two text sizes (body and caption); unread is the bar at its start and Semibold,
  nothing more.
- Side panes are 12 points in from the window's edge.
- Settings apply at once, with no Save. One kind of control per kind of choice: a switch (saying
  On or Off) for on and off, segments for two to four choices, a drop-down for more. Options is in
  parts in the File rail, with Find a setting over them.
- There's no status bar: what needs you shows as a bar over the panes while it lasts.

## Checking a change

A change to the interface is looked at in the real app, light and dark, and in German (the longest
labels), as well as tested: `crates/ui-egui/src/tests.rs` draws every pane and dialog headless, and
the tests read what's drawn (`drawn_text`) and what a screen reader is told.
