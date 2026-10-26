# UI: font, widgets and HUD (`engine::ui`)

Everything 2D in VELOCIDE (menus, lobby, HUD) is drawn by `engine::ui` into a
`gfx::UiDrawList`. The renderer draws that list on top of the upscaled 3D image. The
toolkit is immediate-mode, has no winit dependency and keeps no hidden globals: the game
owns a `UiState` and a `Theme`, builds a `UiInput` from its engine input every frame, and
calls widget functions.

| module | what |
|---|---|
| `ui::font` | SHRIKE 5x7, the built-in pixel font: `build_atlas()`, `glyph_uv()`, metrics |
| `ui::text` | measure, wrap and draw text (`TextStyle`, `Align`, `UiDrawExt`) |
| `ui::widgets` (re-exported) | `Ui` frame builder and all menu widgets |
| `ui::hud` | health bar, stamina pips, style meter, text feeds, banner, crosshair, chat box |
| `ui::draw` | `Rect`, fills, borders, slanted slabs, clipping |
| `ui::theme` | `Theme` palette and metrics |
| `ui::input` / `ui::state` | `UiInput`, `UiKey`, `UiState`, `Id` |
| `ui::raster` | CPU rasteriser (tests, screenshots without a GPU) |

`cargo run -p engine --example ui_demo -- [out_dir]` renders the settings menu (one PNG per
tab), the main menu, a lobby/join screen, the HUD and two font sheets to
`target/ui_demo/` (or `out_dir`). Use it to check UI changes.

## Canvas

The canvas is `UI_CANVAS_HEIGHT` = 360 units tall; its width follows the window aspect
(`UiDrawList::canvas_for_window(w, h)`, 640 at 16:9). One unit is `window_height / 360`
pixels (2 px at 720p, 3 px at 1080p). Origin top-left, +y down. Positions are snapped to
whole units where it matters (text line origins, panel rects) so pixel art stays crisp.

## Font: SHRIKE 5x7

An original font made for this game. Glyphs are 5 px of ink in a 6 x 9 cell: cap height 7,
lowercase x-height 5, 2-px descenders (g j p q y and `_`), 1 empty spacing column on the right.
Round letters have their top-left and bottom-right corners notched (O, C, G, S, 0, 6, 8, 9, o,
c, e, s...), which gives the face a slight forward lean while staying square. Zero is slashed,
and `I l 1 |` and `O 0 o` are all distinct. All printable ASCII 32..=126 is covered; any other
character draws a hatched box.

| constant | value | meaning |
|---|---|---|
| `GLYPH_W` | 6 | quad width at scale 1 (includes the spacing column) |
| `GLYPH_H` | 9 | quad height at scale 1 (includes descenders) |
| `GLYPH_ADVANCE` | 6 | monospaced advance |
| `CAP_HEIGHT` | 7 | rows above the baseline |
| `GLYPH_SPACING` | 1 | trailing empty column (excluded from measured widths) |

**Renderer contract:** `build_atlas()` returns a 128 x 66 RGBA texture (white ink, alpha 255,
transparent elsewhere). `glyph_uv(c)` returns the UV rect of the 6 x 9 cell. Sample it with
**nearest** filtering. Every cell has a 1-px transparent gutter, so slightly-off UVs at
non-integer scales never bleed in a neighbour's pixels (this is tested).

Glyphs are defined in `font.rs` as ASCII-art "sheets" (`#` = ink), several glyphs side by
side, so they can be edited visually. Tests check that every character is defined exactly once,
that all glyphs are distinct, that caps stay above the baseline, and that the atlas matches.
`examples/ui_demo` writes `font_zoom.png` (every glyph at 3x with a baseline marker) and
`font_specimen.png`.

## Text

```rust
use engine::ui::{Align, TextStyle, UiDrawExt};
use engine::ui::text::{draw_text, line_width, text_size, wrap_text, truncate};

let style = TextStyle::new(theme.bone)      // colour
    .scale(2.0)                              // whole numbers stay crisp
    .align(Align::Right)                     // pos.x is the right edge (.centered(), .right())
    .outline(theme.ink)                      // 1 font pixel, 8 directions (HUD readability)
    .shadow(theme.shadow);                   // 1-unit drop shadow (.shadow_by(c, units))
let bounds = list.text(vec2(630.0, 8.0), "PING 42\nFPS 144", &style); // multi-line ok
list.text_wrapped(pos, long_message, 200.0, &TextStyle::new(theme.bone_dim));
list.text_in(rect, "CENTRED", &TextStyle::new(theme.bone).centered()); // v-centred on caps
```

- `line_width(s, scale)` and `text_width(s, scale)` (widest line) return *ink* widths:
  `(chars * 6 - 1) * scale`. `text_size` also returns the height (lines and gaps).
- `wrap_text(text, max_width, scale) -> Vec<&str>`: greedy word wrap that keeps explicit
  `\n`, drops spaces at break points and hard-breaks words longer than a line.
- `truncate(text, max_width, scale)` cuts the text and ends it with `..`.
- `TextStyle::line_gap` is the extra space between lines in font pixels (default 2, so the line
  height is 11 at scale 1).
- Shadows and outlines for a whole string are emitted before its glyphs, so neighbouring
  letters never cover each other.

## Theme

`Theme::default()` is crimson / bone / black. Colours are **sRGB-encoded** `Vec4`s (built with
`theme::rgb(r, g, b)` / `rgba`).

| field | use |
|---|---|
| `crimson` `#A8121C` | accent: title bars, selected tabs, slider fill, active fire mode |
| `crimson_bright` `#EE262C` | focus bar, `>` marker, text cursor, warnings |
| `crimson_dark` `#46080C` | pressed buttons, title bar backing, coloured shadows |
| `bone` `#EEE4CE` / `bone_dim` `#AA9C88` / `ash` `#665A54` | primary / secondary / disabled text |
| `ink`, `panel` (86% alpha), `panel_edge`, `widget`, `widget_hover`, `focus_bg`, `shadow` | surfaces |
| `health`, `health_low`, `hard_damage`, `stamina`, `stamina_empty` | HUD |
| `rank_colors[8]` | DULL ash grey, CRUDE brass, FERAL orange, VICIOUS hot orange, MERCILESS hot red, GRISLY magenta, SEARING red-hot (throbs), VELOCIDE white-hot (flickers red) |

Metrics: `row_height` 14, `spacing` 3, `padding` 12, `title_height` 15, `label_fraction` 0.46
(label column width in label + control rows), `blink_period` 0.8 s.

The game doesn't use `Theme::default()` as is: `client::palette::Palette::theme()` swaps the HUD
colours (`health`, `health_low`, `hard_damage`, `stamina`, `stamina_empty`, `crimson_bright`,
`rank_colors`) for the player's colour-vision mode; the standard palette gives exactly the
default theme. See [Accessibility](#accessibility-colour-blind-palette-and-shape-cues).

Look: translucent near-black panels with a 1-unit edge and bone corner brackets; a crimson title
slab with a stepped 45° cut and trailing slashes; focused rows get a translucent red band, a
2-unit red bar and an animated `>` (it nudges and blinks red/bone) while the label slides
right to make room. Buttons get bevel lines and, when focused, a red fill with corner brackets.
Main-menu items (`menu_item`) are big scale-2 entries with a slanted slab and moving speed
streaks.

## Driving the UI each frame

```rust
// Persistent (e.g. in the game struct):
let theme = Theme::default();
let mut ui_state = UiState::new();
let mut ui_list = UiDrawList::default();

// Every frame:
let canvas = UiDrawList::canvas_for_window(win_w, win_h);
ui_list.canvas_size = canvas;
ui_list.clear();

let ui_input = UiInput {
    mouse_pos: UiInput::canvas_from_window(mouse_px, vec2(win_w as f32, win_h as f32)),
    mouse_down: input.mouse_held(MouseButton::Left),
    mouse_pressed: input.mouse_pressed(MouseButton::Left),
    mouse_released: input.mouse_released(MouseButton::Left),
    scroll: input.scroll_lines(),              // + = wheel up / away from the user
    text: input.typed_text().to_string(),      // winit KeyEvent::text, this frame
    keys: ui_keys_from(&input),                // see below, in press order, incl. key repeat
    time: ctx.time,                            // seconds; drives blink / animations
};

// 1. HUD first (plain functions, no Ui needed) ...
hud::health(&mut ui_list, &theme, health_rect, "WICK", hp, 100.0, hard, ctx.time);
// 2. ... then menus on top.
if menu_open {
    let mut ui = Ui::new(&mut ui_list, &mut ui_state, &ui_input, &theme);
    draw_menu(&mut ui, &mut settings);
    if ui.back_pressed() { menu_open = false; }   // Escape not consumed by a widget
}                                                 // dropping `ui` ends the frame
frame_desc.ui = &ui_list;
```

Key mapping (`ui_keys_from`): arrows → `Up/Down/Left/Right`, Enter / keypad Enter → `Enter`,
Escape → `Escape`, Backspace, Delete, Tab, Home, End, PageUp, PageDown. **Gamepad:** d-pad →
arrows, A / Cross → `Enter`, B / Circle → `Escape`, shoulders → `PageUp/PageDown`. Feed OS
key-repeat presses as extra entries so held arrows and Backspace repeat. Only build a `Ui` while
a menu is open: when no menu is shown, hover/focus state is simply not updated.

While a menu or text field is active the game should ignore its own gameplay bindings:
`ui_state.wants_text_input()` (a text field is focused), `ui_state.is_capturing()` (a key bind is
waiting) and `ui_state.mouse_over_ui()` (mouse over a panel or widget last frame).

## Layout

Widgets are laid out top to bottom in the current *layout*. Every `Ui` starts with the whole
canvas as the root layout; normally you open a panel or area:

```rust
ui.panel(Rect::from_center_size(c, vec2(452.0, 326.0)), "SETTINGS", |ui| { ... });
ui.panel_auto(vec2(16.0, 16.0), 284.0, "JOIN BY ADDRESS", |ui| { ... }); // height fits content
ui.area(rect, |ui| { ... });              // layout without a frame (main menu column)
ui.row(3, |ui| { ui.button("APPLY"); ui.button("DEFAULTS"); ui.button("BACK"); });
ui.row_weights(&[3.0, 1.0], |ui| { ui.text_field("##chat", &mut msg, 120, TextFilter::Any); ui.button("SEND"); });
ui.spacing(4.0);
let r: Rect = ui.allocate(20.0);          // custom widget space; then ui.painter() to draw
let free: Rect = ui.available_rect();
```

Closures may return values (`ui.panel(..., |ui| ui.button("OK"))`). Panels scope ids by
their title.

## Widgets

All labels follow one rule: text after `##` is hidden but still part of the id
(`"PLAY##coop"` shows "PLAY"). Label + control widgets put the label in the left
`label_fraction` of the row and the control on the right.

| call | returns | interaction |
|---|---|---|
| `label(text)`, `label_colored(text, color)`, `text(text, &style)` | – | static text (style can align inside the row) |
| `label_wrapped(text)` | – | paragraph wrapped to the layout width |
| `value_row(label, value)` | – | label left, value right (info rows) |
| `heading(text)` / `title(text)` | – | section header with rule / big scale-2 title |
| `separator()` | – | thin rule |
| `chat_line(name, name_color, message)` | – | wrapped chat message (`name:` coloured) |
| `button(label)` | `bool` | press + release inside, or Enter when focused |
| `menu_item(label)` | `bool` | big main-menu entry, same activation |
| `selectable(label, selected)` | `bool` | list row (server list, players) |
| `toggle(label, &mut bool)` | `bool` changed | click, Enter, Left/Right flip |
| `slider(label, &mut f32, min, max, step, format)` | `bool` changed | drag (keeps tracking outside while held), Left/Right step; snaps to `step` |
| `choice(label, &mut usize, &[&str])` | `bool` changed | Left/Right cycle (wraps), click left arrow = previous, elsewhere = next, Enter = next |
| `text_field(label, &mut String, max_len, TextFilter)` | `TextFieldResponse { changed, submitted, focused }` | type when focused; Backspace, Delete, Left, Right, Home, End; Enter = `submitted`; click places the cursor; scrolls horizontally |
| `keybind(label, current_binding_name)` | `KeybindResponse { started, capturing, cancelled, cleared }` | click / Enter starts capture; Escape cancels; Backspace/Delete = `cleared` |
| `tabs(id, &mut usize, &[&str])` | `bool` changed | click a tab, Left/Right while focused (clamped); equal widths, unless a label needs more (then each tab gets its label plus an equal share) |
| `scroll_area(id, height, f)` | `f`'s value | wheel / PageUp / PageDown when hovered or containing focus, draggable scrollbar, contents clipped |
| `scroll_area_sticky(id, height, f)` | `f`'s value | same, but follows the bottom while scrolled to the end and anchors short content to the bottom (chat logs) |

Slider formats: `{}` = value with as many decimals as `step` needs, `{:.N}` = N decimals,
`{%}` = value x 100 (`"{%}%"` shows 0.5 as `50%`), other text is copied (`"{} BIT"`).

`TextFilter`: `Any` (printable ASCII), `Digits` (ports), `Decimal`, `Address` (hostnames,
IPv4/IPv6 and ports: letters, digits, `. : - _ [ ]`), `Name` (letters, digits, space,
`_ - .`), `Custom(fn(char) -> bool)`.

Other `Ui` methods: `set_enabled(bool)` / `enabled_scope(bool, f)` (greyed out,
unfocusable, non-interactive), `default_focus()` (focus the next widget if nothing is focused,
for keyboard and gamepad players entering a menu), `request_focus(label)` (e.g. focus `"##chat"`
when chat opens), `id(label)`, `push_id(x)` / `pop_id()` / `with_id(x, f)` for repeated labels
(one row per player), `interact(id, rect, focusable) -> Response` for custom widgets,
`painter()`, `theme()`, `time()`, `canvas()`, `state()`, `back_pressed()`, `finish_capture()`.

### Focus and navigation

- Up / Down / Tab move focus through the focusable widgets in layout order, wrapping around.
  They use the previous frame's order, so the new focus is visible in the same frame. With
  nothing focused, Down focuses the first widget and Up the last.
- Moving the mouse onto a widget focuses it (applied at the end of the frame, so two widgets
  are never drawn focused). A resting mouse doesn't steal focus back from the keyboard.
  Pressing on a widget focuses it immediately.
- Enter activates the focused widget; Left / Right adjust sliders, choices, toggles and tabs.
- Focus moved by keys inside a scroll area scrolls the widget into view.
- If the focused widget isn't drawn in a frame, focus is dropped.
- Escape is only consumed when it cancels a key capture; otherwise `ui.back_pressed()` is
  true and the game decides what "back" means. Text fields don't eat Escape.

### Key rebinding flow

```rust
let r = ui.keybind("DASH", &bindings.dash.display_name());
if r.capturing {
    // Ready since the frame after activation, so the click/Enter that started it isn't bound.
    if let Some(b) = input.first_pressed_binding() {   // key, mouse or gamepad button
        bindings.dash = b;
        ui.finish_capture();
    }
}
if r.cleared { bindings.dash = Binding::None; }
```

While capturing, the UI ignores every key except Escape (which cancels and is *not* reported by
`back_pressed`) and blocks mouse interaction with other widgets. The game can also resolve the
capture after building the frame: `if let Some(id) = ui_state.capture_target() { ...;
ui_state.finish_capture(); }`. If the capturing widget stops being drawn, the capture is dropped.

## HUD helpers (`ui::hud`)

Plain functions drawing into the list. Pass `ctx.time` for animations.

```rust
hud::health(list, theme, rect, "WICK", hp, max_hp, hard_damage, time); // big number + 10-hp segments,
                                                                         // hatched hard-damage tail, low-hp blink
hud::segmented_bar(list, rect, value, max, hard, segments, &BarStyle::health(theme)); // the generic bar
hud::pips(list, theme, pos, vec2(14.0, 5.0), 4.0, charges /* e.g. 2.4 */, 3);       // slanted stamina pips
let label_x = pos.x + hud::pips_width(vec2(14.0, 5.0), 4.0, 3) + 6.0;
hud::weapon(list, theme, bottom_right, 2, "BELLOWS", &["GUST", "BOTH BARRELS"], active_mode);
hud::style_meter(list, theme, rect, rank_index, "MERCILESS", fraction, time); // rank colour + meter + rank ticks
hud::meter(list, theme, rect, label, label_scale, fraction, color, Align::Right); // generic
hud::banner(list, theme, center, "MERCILESS", age, 1.6, theme.rank_color(4)) -> bool; // pop 8x -> 4x, fades out
hud::crosshair(list, canvas_center, &CrosshairStyle::new(theme));
hud::chat_box(list, theme, rect, &chat_lines, time, chat_open, 8.0);   // fades old lines when closed
hud::readout(list, theme, pos, Align::Right, "PING", "42", hud::ping_color(theme, 42.0));
hud::format_timer(seconds) // "MM:SS.cc"
```

`TextFeed` is the one stateful helper (the game owns it): `TextFeed::new(max_entries, lifetime)`,
`push(text, color, time)` (repeats of the newest line become `+ PARRY x2`), `update(time)` to drop
faded entries, `draw(list, theme, anchor, align, scale, time)`. Entries slide in, flash white
for a moment and fade out over `fade` seconds. `newest_on_top` controls the stacking direction:
true for the style feed (under the meter), false for a kill feed growing up from its anchor.

## Accessibility: colour-blind palette and shape cues

Settings > **ACCESSIBILITY** (main menu and pause menu, applies live, saved in the settings
file's `accessibility` section): COLOUR MODE Off / Protanopia / Deuteranopia / Tritanopia,
FILTER STRENGTH 0–100 % (the full-screen correction of the 3D view, `docs/render.md`), SHAPE
CUES AUTO / ON / OFF (AUTO = on with any colour mode), and a live preview of the palette.

The UI is drawn after the post pass and is **not** filtered. It uses a colour-blind-safe
palette instead: `client::palette` names every colour that carries meaning by role, with one
variant per mode. `palette()` follows the setting (a process-wide mode, set by
`Common::apply_accessibility`, so code anywhere can call `palette().telegraph` or
`palette().team(i)`), `Palette::for_mode(m)` gets a specific one, `Palette::theme()` builds the
UI theme. `player_color(id)` returns `palette().team(id)`.

| role | meaning | standard | protanopia | deuteranopia | tritanopia |
|---|---|---|---|---|---|
| `team[8]` | player slots (trim, name tags, chat, scoreboard) | red, sky, amber, green, violet, pink, aqua, pale yellow | salmon, blue, yellow, pale aqua, magenta, rose, sky, pale yellow | raspberry, blue, amber, pale green, periwinkle, pink, aqua, yellow | red, light blue, yellow, green, purple, lilac, lime, magenta |
| `telegraph` | enemy wind-up you dodge | #FF2614 | #8C4CFF | #8C4CFF | #3373FF |
| `parry_window` | parryable window (strobe) | #FFE633 | #BFF2FF | #BFF2FF | #FFF299 |
| `weak_point` | glowing weak points | #FF9940 | #FFEB8C | #FFEB8C | #FF8C99 |
| `aim_laser` | Heron aim laser | #FF1F1A | #6673FF | #6673FF | #3373FF |
| `hostile_projectile` | enemy projectiles | #FF8C26 | #BF59FF | #BF59FF | #40BFFF |
| `deflected` | your parried projectile | #FFF7E6 | #FFF7E6 | #FFF7E6 | #FFF7E6 |
| `brand` | Quill brand sigil | #FF4CD9 | #CCFF33 | #CCFF33 | #FF4CD9 |
| `friendly` / `hostile` | team-mode markers | #61C7FF / #ED262B | #4CA6FF / #FF8C0D | #4CA6FF / #FF8C0D | #33D999 / #ED262B |
| `heal` / `damage` | kindling, `+N` / `-N` popups | #FF9E38 / #EE262C | #FFCC33 / #FF6E28 | #FFD940 / #F03C1E | #FFCC73 / #FF2828 |
| `flash_damage` | hit flash | #E60D0A | #FF4C00 | #F23300 | #E60D0A |
| `flash_parry` / `_shield` / `_death` / `_heal` | other flashes | white / pale blue / dark red / faint red (like damage) | same, death #803300, heal gold #FFCC33 | as protan | same, heal #FFCC73 |
| `hard_damage` | smothered capacity (hatched) | #781018 | #965AFF | #965AFF | #3C8CC8 |
| `health_low` / `warning` | low health, DOWNED, LOCKED (`crimson_bright`) | #EE262C | #FF6E28 | #F03C1E | #FF2828 |
| `stamina` | dash pips, good news (READY, CLEARED) | #60C8FF | #60C8FF | #60C8FF | #60C8FF |
| `ranks[8]` | style ranks | grey, brass, orange, hot orange, red, magenta, bright red, white-hot | grey, brass, gold, orange, blue, ice, pink, white | same as protan | grey, brass, orange, red, green, magenta, ember, white |

**Why these:** for protanopes and deuteranopes reds, oranges, browns and molten slag are all one
yellow-brown, which is most of a foundry level. So enemy signals (wind-up, laser, projectiles)
go violet-blue and the parry window white-cyan, the hues they see best that the levels don't
use; dark reds that protanopes barely see (hard damage, low health) move to violet / orange.
Tritanopes keep red-green but lose blue-yellow, so their signals go azure (seen as teal)
against the red foundry, the parry window pale yellow-white, hard damage teal.

**World art.** The standard palette keeps the art's own colours: every Mould and effect has its
own shade of red / orange / yellow, often an HDR multiplier (`Palette::recolours_world()` is
false; the "standard" column above is the nominal role, used by the UI). The colour-blind
palettes replace them by role (`Palette::effect` for particles / sprites / beams,
`Palette::light` for lights, `Palette::model_glow` → `Tint::glow_color` / `core_color` for
model parts, see `docs/models.md`):

| what | role |
|---|---|
| Moulds' `Glow` parts (Gnawer throat, Censer flames + embers, Harrier glare, Gaoler rim + chain, Heron lens ring), wind-up glints (`Fx::EnemyAttack`), Censer whirl embers, Gaoler sweep sparks and arc | `telegraph` |
| any enemy with `PARRYABLE` (Gaoler bash window, the whole Harrier dive): its `Glow` parts strobe full / 55 %; the Harrier dive streak; the wind-up glint of a parryable attack | `parry_window` |
| Moulds' `Core` parts (weak points: Gaoler grille + lantern, Censer coals, Harrier heart, Heron lens, Gnawer eyes) | `weak_point` |
| Heron aim laser while tracking (beam, lens / end glow, light); **locked stays white-hot and wider in every palette** | `aim_laser` |
| ember bombs and needles (model glow, needle streak, trail embers, lights, impact / muzzle glows) | `hostile_projectile` |
| brand sigil, "Branded" glow, the HUD's BRANDED warning | `brand` |
| deflect / parry sparks glow, parried trail | `deflected` |
| shield (spawn protection) and death flashes (`Feedback::shield_flash` / `death_flash`) | `flash_shield`, `flash_death` |

Model glows recoloured this way are drawn at their pose brightness capped at full, so the hue
survives where the art's ×2–3 multipliers would turn it white. Heal visuals (kindling stream,
heal flash, ember pool) are wired by the embers work, not listed here.

**Tests** (`client::palette::tests`): each palette is checked *as its viewer sees it*
(Machado 2009 simulation) with CIEDE2000: HUD pairs (health / hard damage / empty bar / low
health, good / warning, heal / damage) ≥ 15–25, flashes ≥ 20, world pairs (telegraph / parry
≥ 25, hostile / deflected ≥ 20, friendly / hostile ≥ 25, weak point, brand, heal) ≥ 12–25 —
world colours also after the full-screen correction at strength 0, 0.5 and 1 — enemy
signals ≥ 20 from every foundry colour, all 28 team-colour pairs ≥ 12 (about the best eight
bright colours manage for a dichromat; shapes do the rest) and readable on dark panels,
neighbouring ranks ≥ 8. The standard palette is checked with normal vision and must equal the
original look (`STANDARD.theme() == Theme::default()`).

**Shape cues** (`client::ingame::cues`, on when `palette::shape_cues()`):

- **Player markers**: slot `i % 8` gets a 5x5 shape (`palette::MarkerShape`: square, diamond,
  triangle, ring, cross, inverted triangle, bars, chevron) wherever its colour swatch was — name
  tags, scoreboard, lobby list, results, the co-op team list (`cues::player_swatch`).
- **Parry icon**: a `!` diamond strobing between `parry_window` and white over every enemy whose
  `enemy_flags::PARRYABLE` is set (`cues::parry_icons`), above its head or held at the top of the
  view (below the timer) when a big enemy is too close.
- **Health popups**: `+12` (heal colour) / `-8` (damage colour) rising over the health number
  (`Hud`), so heal vs hurt reads from the sign.
- Hard damage is hatched in every mode; low health blinks; hit markers differ in size and
  thickness; the lobby says READY in words.

Check the look with `cargo run -p client --release --example a11y_shots -- out/` (every mode,
plus "as that viewer sees it" comparison strips).

## CPU rasteriser

`ui::raster::rasterize(&list, &font::build_atlas(), w, h) -> TextureData` (transparent
background) or `rasterize_onto(&list, &atlas, &mut image)`. It uses the renderer's mapping
(canvas scaled to pixels, a pixel is covered when its centre is inside a quad), nearest
sampling and straight-alpha "over" blending in sRGB space. `UiTexture::Texture` quads are drawn
magenta.

## Known limitations

- No scissor in `UiDrawList`: scroll areas clip on the CPU by cutting quads (and their UVs) to
  the view rect. Arbitrary custom quads inside a scroll area are clipped the same way.
- Typed text and key presses arrive as separate lists, so their relative order within one frame
  is lost (e.g. "a", Backspace, "b" in one frame).
- Text fields are single-line; no selection, clipboard or IME. Only printable ASCII can be typed
  (the font has nothing else).
- Tab moves focus forward only (`UiInput` has no modifier state for Shift+Tab).
- Panels don't stack or overlap-test: if two panels overlap, both get mouse input.
- Ids are hashes of labels; two identical labels in the same id scope share state (use `##` or
  `with_id`).
