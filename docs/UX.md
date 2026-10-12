# UI/UX design

How the SoftLadder editor should look and behave, benchmarked against the tools people actually
program PLCs with. This is the spec the `softladder-ui` implementation follows; when the code and
this document disagree, one of them is a bug.

Reference tools: **Siemens TIA Portal**, **Rockwell Studio 5000 / RSLogix**, **CODESYS**,
**Beckhoff TwinCAT 3**, **Mitsubishi GX Works**, **Omron Sysmac Studio**, **Schneider EcoStruxure
Control Expert**, **AutomationDirect Productivity Suite**, **Phoenix Contact PLCnext Engineer**.

## 1. What the vendors agree on

These are the conventions every one of those tools shares. They are not taste; they are what a
maintenance technician who has used one PLC brand expects to find.

| Convention | Why it matters | Who does it |
| --- | --- | --- |
| **Project tree** on the left (device → program → blocks/tags/HMI) | It is the map of the project; editing never starts from a flat list of rungs | all |
| **Tabbed document area** (ladder, tags, watch, HMI, diagnostics open side by side) | You flip between the program, its tags and live values constantly | all |
| **Tags are first class** — name, data type, address, comment — and the ladder shows **names**, not addresses | `%I0` is meaningless on a machine; `start_button` is not | all |
| **Network/segment headers** with number, title and comment, always visible | The comment *is* the documentation of the rung | TIA, GX Works, Control Expert |
| **Paper-like ladder rendering** on a light background with a grid | It is a schematic; it must look like one, and printed copies must work | all |
| **Live state on the diagram**: energised = green and continuous, de-energised = grey/blue, values inline | The whole point of online mode is reading the diagram | all |
| **A palette/toolbox grouped by family** (bit logic, timers, counters, compare, math, program control), with icons and shortcuts | 40 flat buttons are unusable | all |
| **Watch & force table** with monitor and *modify* columns, force on/off per row | Commissioning is the job | all |
| **Context menus and keyboard-first editing** (insert before/after, branch, cut/copy/paste, go to tag) | Speed matters; the mouse alone is slow | all |
| **Status bar with run state, online/offline, forced-values indicator, diagnostics count** | Safety: you must never guess whether you are online or forcing | all |
| **Ribbon or grouped toolbar** with labelled groups | Discoverability; shortcuts become learnable | TIA (ribbon), Studio 5000 (tabs), CODESYS (grouped toolbars) |
| **Consistent visual language**: one accent colour, flat surfaces, 1 px borders, small consistent type scale | The tool is stared at for hours | all |

## 2. What we will not copy

* **Online device discovery / download-to-PLC wizards** — we have no target discovery until M7; the
  status bar carries a plain **Simulation** badge instead of an "Online" one.
* **Licence nags, project wizards, 40-step "new device" dialogs.**
* **Modal-everything.** Settings live in docked panes, not in modal dialogs.
* **Proprietary type and data-type editors** beyond what the model supports.

## 3. Shell

```
┌───────────────────────────────────────────────────────────────────────────────────────────┐
│ SoftLadder   File  Edit  Insert  Online  View  Tools  Help                                │  menu
├───────────────────────────────────────────────────────────────────────────────────────────┤
│ ↶ ↷ │ 🗋 🗁 💾 💾+ │ [▶ Run] ⏭ ▤ │ ⊖ ⊕ ⛶ │ % ☾                            2 forced │  command bar
├──────────────────────┬────────────────────────────────────────────────────────────────────┤
│ PROJECT              │ ‹Ladder: Main› ‹PLC tags› ‹Watch & force› ‹Bench› ‹Problems 2›      │  tabs
│  ▾ SoftLadder PLC    ├────────────────────────────────────────────────────────────────────┤
│    ▾ Program         │  Network 1   start_stop                                    ● ok    │
│      ▾ Main          │  Start/stop with self-hold: %I0 starts, %I2 stops.                │
│        1 start_stop  │  ┌──────────────────────────────────────────────────────────────┐  │
│        2 amber_3s    │  │  start_button      stop_button                     green_lamp │  │
│        3 counter     │  │   %I0               %I2                            %Q0        │  │
│    ▸ Subroutines     │  │   -[ ]------+------[/]-----------------------------( )------  │  │
│    ▸ PLC tags        │  │             │                                                  │  │
│    ▸ Watch tables    │  │  green_lamp │                                                  │  │
│    ▸ Simulation      │  │   %Q0       │                                                  │  │
│    ▸ Cross refs      │  │   -[ ]------+                                                  │  │
│                      │  └──────────────────────────────────────────────────────────────┘  │
├──────────────────────┤  Network 2   amber_after_3s                                       │
│ INSPECTOR            │  …                                                                 │
│  Element  Contact NO │                                                                    │
│  Tag      %I0        │                                                                    │
│  Comment  Start      │                                                                    │
├──────────────────────┴────────────────────────────────────────────────────────────────────┤
│ SIM RUN │ cycles 412  scan 0.31 ms │ C:\…\line.slprj * │ 1 error 2 warnings │ 100% │
└───────────────────────────────────────────────────────────────────────────────────────────┘
```

* **Left**: project tree (top pane) and inspector (bottom pane), both resizable, both collapsible.
  Every row has a tooltip with the full name and comment, so a narrow tree does not hide them.
  A trailing number is what the row contains (networks, pages, tags); errors and warnings are
  separate badges. Dragging a ladder element or a tag onto **Watch & force** adds it to the table.
* **Command bar**: one row of 28 pt icon buttons under the menu, grouped by hairlines (history, file,
  run, view). Every button has a tooltip with its name, shortcut and a one-line description. Run/Stop
  is the only labelled button: it is filled in the colour of what it will do, because the run state
  must never be guessed.
* **Palette**: the instruction set sits on top of the ladder document as a compact toolbar. Contacts,
  coils and program control show their glyphs; blocks show their mnemonics (`TON`, `CTU`, `FIFO`).
  Family, name and shortcut letter are in the tooltip, and the armed tool is filled with the accent.
  The palette wraps to a second row on a narrow window and never clips.
* **Centre**: tab bar + the active document. Tabs: one program tab per open section — **Ladder**, or
  **SFC** while a chart section is open — plus **PLC tags**, **Watch & force**, **Bench**,
  **Problems**. The menu shows the project name. A dirty document shows a dot; the tab closes with `×`.
* **Right**: optional secondary pane (cross-reference, tag usage) — off by default.
* **Bottom**: dockable pane for Problems / Watch table / Cross-reference, collapsible, with the
  status bar underneath.
* **Status bar**: one mode pill that cannot be misread — `SIM RUN`, `SIM SCAN`, `SIM STOP`, `FREEZE`
  or `LOAD` — then the cycle count, the last scan time, the project path with `*` when dirty,
  error/warning counts (clickable → Problems), and the canvas zoom. A single scan says `SIM SCAN`;
  it never sits beside an "offline" badge.

## 4. Design tokens

Light theme is the default (all the vendor tools are light; a schematic belongs on paper). Dark is a
toggle, not a separate design.

| Token | Light | Dark | Use |
| --- | --- | --- | --- |
| `surface` | `#F4F5F7` | `#1E1F22` | window background, gutters |
| `panel` | `#FFFFFF` | `#26282C` | panels, tree, inspector |
| `paper` | `#FFFFFF` | `#17181A` | ladder canvas document |
| `paper_grid` | `#E8EAED` | `#2A2C30` | canvas grid |
| `border` | `#D5D8DD` | `#3A3D42` | 1 px separators, panel outlines |
| `text` | `#1B1D21` | `#E6E7E9` | primary text |
| `text_dim` | `#5B6370` | `#9AA0A6` | addresses, units, secondary lines |
| `accent` | `#0F6FC5` | `#4C9AFF` | selection, focus, active tab, links |
| `accent_soft` | `#E3F0FC` | `#1E3A5F` | selected row/network background |
| `energised` | `#00A65A` | `#35C46F` | live power flow, TRUE contacts |
| `wire_idle` | `#5A6472` | `#98A2B3` | de-energised wires and symbols |
| `warning` | `#B26A00` | `#E0A030` | warnings |
| `error` | `#C62828` | `#F26B6B` | errors, broken rungs |
| `run` | `#00A65A` | `#35C46F` | RUN pill |
| `stop` | `#C62828` | `#F26B6B` | STOP pill |

* **Spacing scale**: 4, 8, 12, 16, 24. Nothing uses an off-scale value.
* **Radius**: 4 for controls, 6 for cards/paper, 2 for pills and tags.
* **Type**: 11 caption (addresses, units), 12 body (most UI), 13 emphasis (panel headers, tab
  labels), 20 title (empty-state headings). **Inter** is the interface family and **JetBrains Mono**
  the monospace one, both embedded (`crates/softladder-ui/assets/fonts`, SIL OFL 1.1). Each has a
  semibold family for emphasis (`design::semibold_family`, `design::mono_semibold_family`), since egui
  has no font weights. **Addresses and expressions are monospace** so `%MW100` and `%MW1O0` are
  distinguishable.
* **Contrast**: every text colour meets WCAG AA (4.5:1) on every surface it is drawn on, in both
  themes. `design.rs` tests this.
* **Strokes**: 1 px borders, 2 px wires, 3 px energised wires, 2 px focus ring in `accent`.
* **Icons**: a small hand-drawn vector set (16 px grid): contact NO/NC, edge, coil, set/reset, timer,
  counter, register, compare, operate, connection, jump, call, tag, watch, bench, play, stop, step,
  save, open, new, undo, redo, cut, copy, paste, zoom in/out/reset, grid, cross-reference, warning,
  error, info, folder, plc, lock. Drawn with `egui` paths, no image assets.

## 5. Ladder editor

* **Networks**: each rung is a network with a header band showing `Network <n>`, the rung label
  (title) and the state badge (`ok`, `2 warnings`, `error`). The comment sits under the title in
  `text_dim`. Clicking the header selects the network; double-clicking the title/comment edits it.
* **Element labels**: two lines, tag name above (13 px, `text`) and address below (11 px, monospace,
  `text_dim`). If a variable has no tag, only the address is shown. A View toggle switches to
  address-only.
* **Symbols** are drawn as schematics, not text boxes:
  * NO/NC contacts `-| |-` / `-|/|-` with proper gaps; rising/falling edges `-|P|-` / `-|N|-`.
  * Coils `-( )-`, negated `-(/)-`, set `-(S)-`, reset `-(R)-`, jump `-(J)-`, call `-(C)-`.
  * Function blocks are rectangles with the instance name above, the block type inside, and named
    pins: timer `IN/PT → Q/ET`, counter `CU/CD/R/LD → Q/CV`, register `R/IN/OUT → E/F`.
  * Compare and operate are boxes showing the expression, truncated with an ellipsis and a tooltip
    that shows it in full.
* **Rails**: a left power rail, live while the bench is feeding the network, and a right return rail
  just past the last coil. Each coil wires to that rail. The return rail stays `wire_idle`: it is
  the common, not the power.
* **Blocks stay in their network.** The band reserves the padding the box draws above its first pin
  and below its last, so a counter is never cut by the next network's rule. The instance name sits
  above the parameter (`PT 3000`, `PV 5`), not on one line with it.
* **Live state**: energised wires and pin stubs are `energised` and 3 px; de-energised are `wire_idle`
  and 2 px. A conducting contact or an energised coil is filled, not only stroked. Contacts show the
  tag and, in online mode, a small value chip (`1`/`0`) beside the address, in the same size as the
  label. Labels scale with the document zoom, so a tag does not stay 11 px while its cell shrinks.
  Blocks show `ET/PT` or `CV/PV` values while running. Lamps in the bench mirror `%Q`.
* **Interaction**: hover highlights the cell with a 1 px `accent` outline; the armed palette tool
  shows a ghost preview; click places (replacing, one undo step); drag moves; `Del` deletes; a
  right-click menu offers *Insert contact before / after, Insert branch, Toggle vertical link,
  Delete, Cut, Copy, Paste, Go to tag*; `V` toggles the link; arrows move the selection; `Ctrl+Z/Y`
  undo/redo. The wheel scrolls between networks (`Shift` + wheel goes sideways); there is no
  drag-pan, so the sheet never drifts under the pointer. Zoom with `Ctrl +/-`, `Ctrl/Cmd` + wheel
  or `Ctrl+0`.
* **Block pins**: a block in series has one rung input per row the engine reads on the left
  (`IN`; `R/LD/CU/CD`; `R/IN/OUT`) and a single flow-out on the right (`Q`, or `E` for a
  register) sitting on the first row's wire, which is where the rung continues. Readouts
  (`ET`, `CV`, `F`) are labels with no stub: they are read as variables, never wired.
* **Empty states**: an empty project shows a centred card ("No program yet — **New rung** or open a
  project"); a section with no rungs shows "This section has no rungs"; an unplaced rung shows a
  dashed placeholder "Click a cell to place the armed element".

## 6. PLC tags (the symbol table, promoted)

A first-class table, not a dialog: columns **Name**, **Type**, **Address**, **Comment**, **Used by**
(count, clickable → the watch table). The header stays put, rows are zebra-striped, a filter narrows
them and a column title sorts them. Rows are editable in place. The add bar sits above the rows.
The row menu watches or deletes a tag, and the grip on a bound tag drags it onto Watch & force.
An address that does not parse is refused with the reason in the status note. The canvas, the watch
table and the bench all display the **name** and fall back to the address.

Data types are derived from the variable kind: `Bool` for `%I/%Q/%M/%QLED/%S`, `Int` for
`%MW/%IW/%QW`, `Timer` for `%TM`, `Counter` for `%C`, `Register` for `%R`, `Step` for `%X`.

## 7. Watch & force

A table with **Address/Tag**, **Type**, **Value** (live, monospaced), **Format** (`Bool`, `Signed`,
`Hex`, `Real`), **Modify value** (typed in, applied with a button or `Enter`) and **Force**
(a lock; while it is armed the row and the lock are `warning`, and the status bar shows `N forced`).
The same table chrome as the tag list: fixed header, zebra rows, filter, sort and a row menu
(release, remove, copy the address). The add bar sits above the rows. A row can be added by typing
an address, by picking a tag, by right-clicking an element ("Add to watch table"), or by dragging
the element or its tag onto the Watch tab, the tree row or the drop band. Monitoring runs while the
bench runs; values update in place.

## 8. Bench (the HMI view)

Laid out like an operator screen, not a form. Every widget is a card of the same size, in a grid
that wraps. A tag prefix shared by two or more widgets (`conveyor_run`, `conveyor_speed`) is a
zone titled from that prefix; a name that shares no prefix stays with its role: Inputs, Outputs
or Analog.

Switches are toggles with a visible on/off state. Momentary push-buttons are raised and sink,
with a lit pip, while they are held; they spring back after the scan. Lamps are round pilots:
dark in the bezel when off, and a halo in the lamp's colour when on. The colour is Green, Amber,
Red or Blue, chosen in the inspector, and auto-fill guesses it from the tag name (`amber_lamp`,
`fault_lamp`). Sliders and gauges are real widgets; the readout and the scale print the
engineering unit when the widget has one. Auto-fill copies the unit from the tag.

Under the grid, the trend records every scan. Outputs are pinned until the operator toggles the
chips. The plot's horizontal axis is seconds of simulated time. A new run clears the history.
Editing a widget stays in the inspector: label, momentary, colour, unit and range.

## 9. Feedback, empty states and errors

* A **note line** in the status bar replaces modal "done" dialogs (the current `status` string, but
  styled and transient).
* Problems are surfaced in three places: the rung header badge, the Problems tab with a count in the
  tab label, and the status bar counts. Clicking any of them selects the offending network and cell.
* Destructive actions ask in a **modal dialog with a clear primary button**, never a bare OK/Cancel,
  and never for something undoable.
* Nothing is silent: if an action cannot be done, the reason appears next to the control that failed.

## 10. Accessibility and platform behaviour

* Full keyboard operation: every ribbon button and tree node is reachable with `Tab`/arrows and
  activatable with `Enter`/`Space`; focus is always visible.
* Shortcuts use `Modifiers::command`, so `Cmd` on macOS and `Ctrl` elsewhere.
* HiDPI: everything is laid out in points; the screenshot harness renders at 1× and 2×.
* Font scale respects the OS setting; panels scroll instead of clipping at small window sizes
  (tested at 1024 × 700).

## 11. How the UI is reviewed

There is no display server in CI, so the interface is reviewed through **headless screenshots**:
`crates/softladder-ui/tests/ui_shots.rs` runs the real `EditorApp::draw` on an `egui::Context` and
rasterises the tessellated frame (plus the font atlas) into PNGs under `target/ui-shots/`. Every
visual change is checked against those images before it is committed, and the shot list covers the
states above: opening a project, selecting an element, running, placing an element, each tab, the
dialogs, and a narrow window.

## 12. SFC editor

Sequential Function Chart is the second language, and the vendors give it its own editor rather than
a mode of the ladder one: **TIA Portal GRAPH**, **CODESYS SFC**, **Studio 5000 SFC**, **GX Works
SFC**. Those tools share a look, and this is what we follow.

**A document per section.** An SFC section opens its own tab next to the ladder ones, labelled with
the section name and an `SFC` chip; the project tree shows the same chip on the section node.
Selecting a section in the tree opens the right document for its language.

**The page is the sheet.** A page is drawn on `tokens.paper` with the same sheet, border and grid as
the ladder document, but the grid is square and coarse (a step or a transition occupies one cell, as
in the reference's 32 × 32 pages). A section with several pages shows them as bands down the sheet
with the page comment in the header, exactly like the ladder's networks; the page selector in the
inspector jumps between them.

**Steps and transitions are schematic.** A step is a square with its number inside; the **initial
step** is drawn with a doubled border and an `init` chip, the way GRAPH does. A step that is active
while the bench runs is filled in `tokens.energised` with its number in white and its elapsed time
(`%X<n>.V`) as a value chip beside it. A transition is a short horizontal bar crossed by the
condition, drawn as a bar plus a hairline, with the condition written beside it (the tag name over
the address, as on the ladder) and a tooltip that shows the whole expression. An **AND divergence or
convergence** is drawn with the double bar of the IEC notation, an OR with the single one, so the two
are distinguishable at a glance — this is where the vendors' drawings earn their keep.

**Links are drawn as wiring.** Vertical lines connect a step to the transitions it feeds and a
transition to the steps it activates, on the same sheet grid, with the same idle/energised strokes as
the ladder (`tokens.wire_idle` 2 px / `tokens.energised` 3 px). Links are not placed by hand: the
editor derives them from the model's step and transition positions, so the drawing can never
disagree with what the engine will run.

**Palette.** The ribbon's Insert group changes with the document: *Initial step*, *Step*,
*Transition*, *Link*, *AND divergence*, *OR divergence*, *Comment*. Placement is the ladder's —
click the palette chip, click the cell, replace on an occupied cell as one undo step — and every
edit is a `softladder-edit` command, so undo/redo, the dirty mark and the Problem list work exactly
as they do on the ladder.

**Inspector.** With a step selected: number, initial flag, page and position, and its activity and
timer while running. With a transition selected: the condition (a variable field with the same
validation and tag picker as the ladder, or an expression), and the steps it activates and
deactivates. With a page selected: the comment.

**Live state and validation.** While the bench runs, active steps are filled, true transitions are
emphasised, and step times update in place. Diagnostics land in the Problems document with a
location (`page n · step m`), and clicking a row opens the SFC page and selects the element — the
same gesture as the ladder, because it is the same panel.

**As built — the one place the model forced a decision.** A transition is a junction when it has
more than one source or more than one target, and that is what the double bar means. The *AND
divergence* tool inserts a transition wired to the whole parallel group of the column above and
below; the *OR divergence* tool wires the nearest step above and below, so a second OR branch placed
the same way shares the source and the inspector labels the junction `OR`. A plain sequence
transition gets no badge rather than a wrong one.

**Not in the first cut:** macro steps, variable-based step numbers, action qualifiers as separate
boxes, and printing an SFC page. They are recorded here so the first cut is judged against a list
rather than against a memory.
