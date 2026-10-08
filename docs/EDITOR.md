# Editor

The desktop editor is `softladder-ui` (egui/eframe). It is deliberately thin: all it does is draw
[`softladder-edit`](../crates/softladder-edit)'s `Editor` and `Bench` and turn input into `Command`s.
Anything described here that changes the program is a method on those types, which is why the editor
is covered by headless tests.

## Layout

The window follows the convention every industrial programming tool shares: a ribbon of labelled
command groups, a project tree on the left, **documents** in the middle, an inspector on the right
and a status bar of badges. [`UX.md`](UX.md) §3 has the full specification and §4 the design tokens.

```
┌──────────────────────────────────────────────────────────────────────────────────┐
│ SoftLadder   File  Edit  Insert  Online  View  Tools  Help                       │
├──────────────────────────────────────────────────────────────────────────────────┤
│ ↶ ↷ │ New Open Save │ Run Step Fill │ − + 1:1 Addresses Dark │ NO NC TON CTU …  │
├──────────────────┬───────────────────────────────────────────┬───────────────────┤
│ PROJECT          │ ‹Ladder› ‹PLC tags› ‹Bench› ‹Watch & force› ‹Problems›        │
│  traffic_light   ├───────────────────────────────────────────┤ INSPECTOR         │
│   Program        │  Network 1   start_stop             ok    │  Element          │
│    Main LAD      │  Start/stop with self-hold…               │  Contact NO       │
│     1 start_stop │   start_button      stop_button           │  Tag  %I0         │
│     2 amber_3s   │    %I0               %I2                  │  Params           │
│   PLC tags       │    -[ ]------+------[/]------( )-         │                   │
│   Simulation     │              │                            │                   │
│   Watch & force  │   green_lamp │                            │                   │
│   Problems       │    %Q0       │                            │                   │
├──────────────────┴───────────────────────────────────────────┴───────────────────┤
│ ● Simulation │ RUN │ cycles 412 │ scan 0.31 ms │ line.slprj * │ 1 error 2 warn │ 100% │
└──────────────────────────────────────────────────────────────────────────────────┘
```

* **Left** — the project tree: the PLC, the program, one node per section, its rungs (`1 start_stop`
  with a one-line comment and an error dot), then the PLC tags, the bench, the watch table and the
  problems. Clicking a rung selects it and opens the ladder; clicking a node opens that document.
* **Centre** — the open document. **Ladder** draws the selected section as numbered networks;
  **PLC tags** is the tag table; **Bench** is the simulation panel laid out as an operator screen;
  **Watch & force** monitors and forces variables; **Problems** lists the diagnostics.
* **Right** — the inspector: whatever is selected (an element, a rung, a section, a bench widget),
  with the tag field, its validation, the parameters, the vertical link and the related commands.
* **Bottom** — the status bar: simulation badge, run state, cycles, scan time, the project path with
  `*` when dirty, clickable problem counts and the zoom.

## Placing and editing

1. Pick an element in the palette (or press its letter shortcut).
2. Click a cell to place it. Placing on an occupied cell replaces the element (one undo step).
3. With the pointer tool, click an element to select it; the properties strip shows its variable and
   parameters. An invalid variable is refused with a message and the element keeps its old value.
4. Drag a selected element to another cell to move it. `Delete` removes it.
5. `V` toggles the vertical link of the selected cell, which is how parallel branches and their merge
   points are drawn — see [`SEMANTICS.md`](SEMANTICS.md) §2.

Variable fields accept every ClassicLadder spelling (`%I0`, `%B3`, `%TM0.P`, `%C1.V`, `%R0.I`,
`%MW0.3`) and show the canonical form; autocompletion lists the variables and symbols already used in
the project before inventing new ones.

## Keyboard

| Shortcut | Action |
| --- | --- |
| `Ctrl/Cmd + Z` | Undo |
| `Ctrl/Cmd + Shift + Z`, `Ctrl/Cmd + Y` | Redo |
| `Ctrl/Cmd + N` | New project |
| `Ctrl/Cmd + O` | Open |
| `Ctrl/Cmd + S` | Save |
| `Ctrl/Cmd + Shift + S` | Save as |
| `Ctrl/Cmd + R` | Run / Stop |
| `Ctrl/Cmd + T` | Single scan |
| `Ctrl/Cmd + Shift + A` | Auto-fill the bench from the program |
| `Delete` / `Backspace` | Delete the selection |
| Arrows | Move the selection one cell |
| `Ctrl/Cmd + =` / `Ctrl/Cmd + -` | Zoom in / out |
| `Ctrl/Cmd + 0` | Reset the view |
| `F1` | Shortcut help |
| `V` | Toggle the selected cell's vertical link |

## Bench

The bench is a stand-in for the machine. Its layout belongs to the project (`Project::simulation`)
and is saved with it; the operator's positions do not (they are runtime state in `PanelState`).

- **Switches** drive `%I` inputs; a widget marked *momentary* springs back after one scan, so it
  behaves like a real push-button.
- **Lamps** follow `%Q` outputs.
- **Sliders** drive `%IW` values, **gauges** follow `%QW` values, both clamped to their range.
- *Auto-fill* builds a widget for every physical variable the program uses and labels it with the
  bound symbol, so a project without a bench is one click from being commissionable.

While running, each cycle applies the bench to the input image, scans once with the simulated clock,
releases momentary buttons and reads the outputs back. Stopping freezes the picture; *single scan*
advances exactly one cycle.

## Live indication

- Energised wiring and contacts are drawn brighter and thicker; de-energised elements stay dim, so
  the state of the program can be read at a glance.
- Open and closed contacts are drawn differently (`-[ ]-` vs `-[/]-`), as are latched coils.
- Timers show their elapsed and preset time in time-base units, counters show value and preset,
  registers show how many values they hold.
- The selected rung is the one the properties strip and the problems list refer to.
- Errors mark the rung with a red edge; clicking a problem selects the offending rung and cell.

## Not in M2

Multi-selection and rubber-band edit, drag & drop between rungs, copy/paste of rung fragments,
cross-references, rename-in-project, symbol autocompletion across the whole project, i18n and
theming, PNG/SVG/PDF export, and the scope/trend view. See [`PLAN.md`](PLAN.md) for the milestone
each belongs to.
