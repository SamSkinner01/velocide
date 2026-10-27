//! Game actions, their bindings, and turning engine input into [`PlayerInput`].
//!
//! - [`Action`]: every rebindable action, with menu labels and default bindings.
//! - [`Bindings`]: action → bindings (several per action: typically one keyboard/mouse binding
//!   and one gamepad binding), serialised in the settings file.
//! - [`WeaponSelect`]: the weapon selection rule (slot key = select, same slot again = cycle
//!   the build's ability, wheel = next/previous weapon; empty loadout slots are skipped).
//! - [`Controls`]: per-session input state. Mouse look is applied **every frame**
//!   ([`Controls::look`]) so the camera is as responsive as the display allows; button presses
//!   are latched every frame ([`Controls::latch`]) and the input sent each fixed tick
//!   ([`Controls::build`]) carries the latest view angles plus every button that was held or
//!   tapped since the previous tick.

use std::collections::BTreeMap;

use engine::input::{
    AxisDirection, Binding, GamepadAxis, GamepadButton, InputState, Key, MouseButton,
    WheelDirection,
};
use serde::{Deserialize, Serialize};
use shared::input::{Buttons, PlayerInput};
use shared::protocol::MAX_PITCH;

use crate::settings::Settings;

/// Every rebindable action.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Action {
    MoveForward,
    MoveBack,
    MoveLeft,
    MoveRight,
    Jump,
    /// Slide on the ground, slam in the air.
    Crouch,
    Dash,
    Fire,
    AltFire,
    /// Punch / parry.
    Punch,
    /// Throw a thorn (older settings files call it `ThrowPrism`).
    #[serde(alias = "ThrowPrism")]
    ThrowThorn,
    Use,
    Weapon1,
    Weapon2,
    Weapon3,
    Weapon4,
    NextWeapon,
    PrevWeapon,
    /// Cycle the current weapon's alt-fire mode (gamepads; keyboard players press the slot key
    /// again).
    NextMode,
    /// Hold: scoreboard. In the warm-up: opens / closes the lobby overlay.
    Scoreboard,
    Chat,
    Pause,
    Restart,
    DebugOverlay,
    Fullscreen,
    /// Toggle ready in the lobby / the warm-up without opening the lobby (docs/lobby.md).
    Ready,
}

impl Action {
    pub const ALL: [Action; 26] = [
        Action::MoveForward,
        Action::MoveBack,
        Action::MoveLeft,
        Action::MoveRight,
        Action::Jump,
        Action::Crouch,
        Action::Dash,
        Action::Fire,
        Action::AltFire,
        Action::Punch,
        Action::ThrowThorn,
        Action::Use,
        Action::Weapon1,
        Action::Weapon2,
        Action::Weapon3,
        Action::Weapon4,
        Action::NextWeapon,
        Action::PrevWeapon,
        Action::NextMode,
        Action::Scoreboard,
        Action::Chat,
        Action::Pause,
        Action::Restart,
        Action::DebugOverlay,
        Action::Fullscreen,
        Action::Ready,
    ];

    /// Label in the controls menu.
    pub fn label(self) -> &'static str {
        match self {
            Action::MoveForward => "MOVE FORWARD",
            Action::MoveBack => "MOVE BACK",
            Action::MoveLeft => "STRAFE LEFT",
            Action::MoveRight => "STRAFE RIGHT",
            Action::Jump => "JUMP",
            Action::Crouch => "SLIDE / SLAM",
            Action::Dash => "DASH",
            Action::Fire => "FIRE",
            Action::AltFire => "ALT FIRE",
            Action::Punch => "PUNCH / PARRY",
            Action::ThrowThorn => "THROW THORN",
            Action::Use => "USE / REVIVE",
            Action::Weapon1 => "WEAPON SLOT 1",
            Action::Weapon2 => "WEAPON SLOT 2",
            Action::Weapon3 => "WEAPON SLOT 3",
            Action::Weapon4 => "WEAPON SLOT 4",
            Action::NextWeapon => "NEXT WEAPON",
            Action::PrevWeapon => "PREVIOUS WEAPON",
            Action::NextMode => "CYCLE FIRE MODE",
            Action::Scoreboard => "SCOREBOARD / LOBBY",
            Action::Chat => "CHAT",
            Action::Pause => "PAUSE",
            Action::Restart => "RESTART (SOLO)",
            Action::DebugOverlay => "DEBUG OVERLAY",
            Action::Fullscreen => "FULLSCREEN",
            Action::Ready => "READY UP (LOBBY)",
        }
    }

    /// Default bindings: keyboard/mouse first, then gamepad.
    pub fn default_bindings(self) -> Vec<Binding> {
        use Binding as B;
        let key = B::Key;
        let pad = B::GamepadButton;
        match self {
            Action::MoveForward => vec![key(Key::KeyW)],
            Action::MoveBack => vec![key(Key::KeyS)],
            Action::MoveLeft => vec![key(Key::KeyA)],
            Action::MoveRight => vec![key(Key::KeyD)],
            Action::Jump => vec![key(Key::Space), pad(GamepadButton::South)],
            Action::Crouch => vec![
                key(Key::ControlLeft),
                key(Key::KeyC),
                pad(GamepadButton::East),
            ],
            Action::Dash => vec![key(Key::ShiftLeft), pad(GamepadButton::LeftBumper)],
            Action::Fire => vec![
                B::Mouse(MouseButton::Left),
                B::GamepadAxis(GamepadAxis::RightTrigger, AxisDirection::Positive),
            ],
            Action::AltFire => vec![
                B::Mouse(MouseButton::Right),
                B::GamepadAxis(GamepadAxis::LeftTrigger, AxisDirection::Positive),
            ],
            Action::Punch => vec![key(Key::KeyF), pad(GamepadButton::RightBumper)],
            Action::ThrowThorn => vec![key(Key::KeyQ), pad(GamepadButton::North)],
            Action::Use => vec![key(Key::KeyE), pad(GamepadButton::West)],
            Action::Weapon1 => vec![key(Key::Digit1)],
            Action::Weapon2 => vec![key(Key::Digit2)],
            Action::Weapon3 => vec![key(Key::Digit3)],
            Action::Weapon4 => vec![key(Key::Digit4)],
            Action::NextWeapon => vec![
                B::Wheel(WheelDirection::Down),
                pad(GamepadButton::DPadRight),
            ],
            Action::PrevWeapon => vec![B::Wheel(WheelDirection::Up), pad(GamepadButton::DPadLeft)],
            Action::NextMode => vec![B::Mouse(MouseButton::Middle), pad(GamepadButton::DPadUp)],
            Action::Scoreboard => vec![key(Key::Tab), pad(GamepadButton::Select)],
            Action::Chat => vec![key(Key::KeyT), key(Key::Enter)],
            Action::Pause => vec![key(Key::Escape), pad(GamepadButton::Start)],
            Action::Restart => vec![key(Key::KeyR)],
            Action::DebugOverlay => vec![key(Key::F3)],
            Action::Fullscreen => vec![key(Key::F11)],
            Action::Ready => vec![key(Key::F1), pad(GamepadButton::DPadDown)],
        }
    }

    /// Weapon slot (0-based) for the slot actions.
    pub fn weapon_slot(self) -> Option<u8> {
        match self {
            Action::Weapon1 => Some(0),
            Action::Weapon2 => Some(1),
            Action::Weapon3 => Some(2),
            Action::Weapon4 => Some(3),
            _ => None,
        }
    }
}

/// Whether a binding comes from a gamepad (the controls menu keeps one keyboard/mouse and one
/// gamepad binding per action side by side).
pub fn is_gamepad_binding(b: &Binding) -> bool {
    matches!(b, Binding::GamepadButton(_) | Binding::GamepadAxis(..))
}

/// Action → bindings. Missing actions (older settings files) get their defaults; an action
/// stored with an empty list stays unbound.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Bindings {
    map: BTreeMap<Action, Vec<Binding>>,
}

impl Default for Bindings {
    fn default() -> Self {
        Self {
            map: Action::ALL
                .iter()
                .map(|&a| (a, a.default_bindings()))
                .collect(),
        }
    }
}

impl Bindings {
    /// Adds defaults for actions missing from a loaded file.
    pub fn fill_missing(&mut self) {
        for a in Action::ALL {
            self.map.entry(a).or_insert_with(|| a.default_bindings());
        }
    }

    pub fn get(&self, action: Action) -> &[Binding] {
        self.map.get(&action).map_or(&[], Vec::as_slice)
    }

    pub fn is_down(&self, input: &InputState, action: Action) -> bool {
        self.get(action).iter().any(|&b| input.is_down(b))
    }

    pub fn pressed(&self, input: &InputState, action: Action) -> bool {
        self.get(action).iter().any(|&b| input.pressed(b))
    }

    /// Binds `binding` to `action`, replacing the action's binding of the same device class
    /// (keyboard/mouse vs gamepad) and removing it from every other action.
    pub fn bind(&mut self, action: Action, binding: Binding) {
        for list in self.map.values_mut() {
            list.retain(|b| *b != binding);
        }
        let pad = is_gamepad_binding(&binding);
        let list = self.map.entry(action).or_default();
        list.retain(|b| is_gamepad_binding(b) != pad);
        if pad {
            list.push(binding);
        } else {
            list.insert(0, binding);
        }
    }

    /// Removes the action's keyboard/mouse bindings (or gamepad bindings with `gamepad`).
    pub fn clear(&mut self, action: Action, gamepad: bool) {
        if let Some(list) = self.map.get_mut(&action) {
            list.retain(|b| is_gamepad_binding(b) != gamepad);
        }
    }

    /// Short name of the action's first binding for on-screen hints ("F1", "TAB"), preferring
    /// keyboard / mouse; `None` when unbound.
    pub fn hint(&self, action: Action) -> Option<String> {
        let list = self.get(action);
        list.iter()
            .find(|b| !is_gamepad_binding(b))
            .or_else(|| list.first())
            .map(|b| b.name().to_uppercase())
    }

    /// Menu text: "Space / Pad A", "-" when unbound.
    pub fn display(&self, action: Action) -> String {
        let names: Vec<String> = self.get(action).iter().map(|b| b.name()).collect();
        if names.is_empty() {
            "-".into()
        } else {
            names.join(" / ").to_uppercase()
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Weapons

/// Abilities (alt-fire modes) per build, cycled by pressing the slot again.
pub const MODES_PER_WEAPON: usize = shared::loadout::BUILD_ABILITIES;
/// Weapon slots on the number keys (`PlayerInput::weapon` = slot index).
pub const SLOTS: usize = shared::loadout::LOADOUT_SLOTS;

/// What a selection input changed (for the switch sound and viewmodel animation).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WeaponChange {
    None,
    Weapon,
    Mode,
}

/// Weapon selection state: the selected slot, and each slot's selected ability. Slots the
/// carried loadout leaves empty are skipped ([`WeaponSelect::set_loadout`]); what the slots
/// hold comes from the loadout, not from here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WeaponSelect {
    pub current: u8,
    pub modes: [u8; SLOTS],
    /// Slots holding a weapon (every slot until a loadout says otherwise).
    pub filled: [bool; SLOTS],
}

impl Default for WeaponSelect {
    fn default() -> Self {
        Self {
            current: 0,
            modes: [0; SLOTS],
            filled: [true; SLOTS],
        }
    }
}

impl WeaponSelect {
    pub fn mode(&self) -> u8 {
        self.modes[self.current as usize % SLOTS]
    }

    /// Follows the carried loadout: its empty slots are skipped, and if the selected slot
    /// emptied the first weapon is selected. Returns whether the selection moved.
    pub fn set_loadout(&mut self, loadout: &shared::loadout::Loadout) -> bool {
        let filled: [bool; SLOTS] = std::array::from_fn(|i| loadout.slots[i].is_some());
        if !filled.contains(&true) {
            return false;
        }
        self.filled = filled;
        if self.filled[self.current as usize % SLOTS] {
            return false;
        }
        self.current = self.filled.iter().position(|f| *f).unwrap_or(0) as u8;
        true
    }

    /// Slot key: selects the slot's weapon instantly; pressing the selected slot again cycles
    /// its ability. An empty slot does nothing.
    pub fn press_slot(&mut self, slot: u8) -> WeaponChange {
        let i = slot as usize;
        if i >= SLOTS || !self.filled[i] {
            return WeaponChange::None;
        }
        if slot == self.current {
            self.cycle_mode();
            WeaponChange::Mode
        } else {
            self.current = slot;
            WeaponChange::Weapon
        }
    }

    pub fn cycle_mode(&mut self) {
        let m = &mut self.modes[self.current as usize % SLOTS];
        *m = (*m + 1) % MODES_PER_WEAPON as u8;
    }

    /// The next slot holding a weapon (wraps).
    pub fn next(&mut self) {
        self.step(1);
    }

    /// The previous slot holding a weapon (wraps).
    pub fn prev(&mut self) {
        self.step(SLOTS - 1);
    }

    fn step(&mut self, by: usize) {
        let mut slot = self.current as usize % SLOTS;
        for _ in 0..SLOTS {
            slot = (slot + by) % SLOTS;
            if self.filled[slot] {
                break;
            }
        }
        self.current = slot as u8;
    }
}

// ---------------------------------------------------------------------------------------------
// Look + input building

/// Degrees of yaw/pitch per mouse count at sensitivity 1 (Quake's `m_yaw`).
pub const MOUSE_DEGREES_PER_COUNT: f32 = 0.022;

/// Applies a raw mouse delta (counts, +y = down) to yaw/pitch. Moving the mouse right turns
/// right (yaw decreases, see `shared::input`), moving it up looks up. Pitch is clamped.
pub fn apply_mouse_look(yaw: &mut f32, pitch: &mut f32, delta: glam::Vec2, settings: &Settings) {
    let k = (settings.mouse_sensitivity * MOUSE_DEGREES_PER_COUNT).to_radians();
    let invert = if settings.invert_y { -1.0 } else { 1.0 };
    *yaw -= delta.x * k;
    *pitch = (*pitch - delta.y * k * invert).clamp(-MAX_PITCH, MAX_PITCH);
}

/// Applies a right-stick deflection (+y = up) for `dt` seconds. A slight response curve gives
/// precision near the centre.
pub fn apply_stick_look(yaw: &mut f32, pitch: &mut f32, stick: glam::Vec2, dt: f32, s: &Settings) {
    let len = stick.length();
    if len <= 0.0 {
        return;
    }
    let curved = stick * len; // quadratic response, same direction
    let rate = s.gamepad_look_sensitivity.to_radians() * dt;
    let invert = if s.gamepad_invert_y { -1.0 } else { 1.0 };
    *yaw -= curved.x * rate;
    *pitch = (*pitch + curved.y * rate * 0.75 * invert).clamp(-MAX_PITCH, MAX_PITCH);
}

/// Buttons sent with each input and the actions that produce them.
const BUTTON_ACTIONS: [(Action, Buttons); 8] = [
    (Action::Jump, Buttons::JUMP),
    (Action::Crouch, Buttons::CROUCH),
    (Action::Dash, Buttons::DASH),
    (Action::Fire, Buttons::FIRE),
    (Action::AltFire, Buttons::ALT_FIRE),
    (Action::Punch, Buttons::PUNCH),
    (Action::ThrowThorn, Buttons::THROW),
    (Action::Use, Buttons::USE),
];

/// Per-session input state: view angles, weapon selection, buttons latched between ticks.
#[derive(Clone, Debug, Default)]
pub struct Controls {
    pub yaw: f32,
    pub pitch: f32,
    pub weapons: WeaponSelect,
    /// Buttons pressed (even briefly) since the last tick.
    latched: Buttons,
    /// The view when a fire / punch button was first pressed since the last tick: the next
    /// tick's input aims there, so the shot goes where the crosshair was on the click's frame
    /// (where its press-frame feedback was drawn, docs/feel.md), not where it drifted by the
    /// tick.
    press_aim: Option<(f32, f32)>,
    /// Movement axes / buttons injected by the autopilot (`--bot`), replacing the devices.
    pub override_move: Option<(f32, f32, Buttons)>,
}

impl Controls {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adopt a view (spawn / respawn).
    pub fn set_view(&mut self, yaw: f32, pitch: f32) {
        self.yaw = yaw;
        self.pitch = pitch.clamp(-MAX_PITCH, MAX_PITCH);
    }

    /// Per frame: mouse and right-stick look.
    pub fn look(&mut self, input: &InputState, settings: &Settings, dt: f32) {
        apply_mouse_look(
            &mut self.yaw,
            &mut self.pitch,
            input.mouse_delta(),
            settings,
        );
        apply_stick_look(
            &mut self.yaw,
            &mut self.pitch,
            input.right_stick(),
            dt,
            settings,
        );
    }

    /// Per frame: remember button presses (so taps shorter than a tick still reach the
    /// simulation) and handle weapon selection. Returns what the selection changed.
    pub fn latch(&mut self, input: &InputState, bindings: &Bindings) -> WeaponChange {
        for (action, button) in BUTTON_ACTIONS {
            if bindings.pressed(input, action) {
                self.latched.set(button, true);
                if matches!(action, Action::Fire | Action::AltFire | Action::Punch) {
                    self.press_aim.get_or_insert((self.yaw, self.pitch));
                }
            }
        }
        let mut change = WeaponChange::None;
        for action in [
            Action::Weapon1,
            Action::Weapon2,
            Action::Weapon3,
            Action::Weapon4,
        ] {
            if bindings.pressed(input, action)
                && let Some(slot) = action.weapon_slot()
            {
                change = self.weapons.press_slot(slot);
            }
        }
        if bindings.pressed(input, Action::NextWeapon) {
            self.weapons.next();
            change = WeaponChange::Weapon;
        }
        if bindings.pressed(input, Action::PrevWeapon) {
            self.weapons.prev();
            change = WeaponChange::Weapon;
        }
        if bindings.pressed(input, Action::NextMode) {
            self.weapons.cycle_mode();
            change = WeaponChange::Mode;
        }
        change
    }

    /// Forget latched presses (e.g. when gameplay input is suppressed by chat or a menu).
    pub fn clear_latched(&mut self) {
        self.latched = Buttons::NONE;
        self.press_aim = None;
    }

    /// Per fixed tick: the input to send. `gameplay` false (chat open, menu) sends a neutral
    /// input that keeps the view angles and weapon selection.
    pub fn build(
        &mut self,
        input: &InputState,
        bindings: &Bindings,
        gameplay: bool,
    ) -> PlayerInput {
        let (yaw, pitch) = self.press_aim.take().unwrap_or((self.yaw, self.pitch));
        let mut out = PlayerInput {
            yaw,
            pitch,
            weapon: self.weapons.current,
            weapon_mode: self.weapons.mode(),
            ..Default::default()
        };
        let latched = std::mem::take(&mut self.latched);
        if !gameplay {
            return out;
        }
        if let Some((forward, right, buttons)) = self.override_move {
            out.forward = PlayerInput::quantize_axis(forward);
            out.right = PlayerInput::quantize_axis(right);
            out.buttons = buttons;
            return out;
        }
        let axis = |pos: Action, neg: Action| {
            (bindings.is_down(input, pos) as i32 - bindings.is_down(input, neg) as i32) as f32
        };
        let stick = input.left_stick();
        let mut forward = axis(Action::MoveForward, Action::MoveBack) + stick.y;
        let mut right = axis(Action::MoveRight, Action::MoveLeft) + stick.x;
        let len = (forward * forward + right * right).sqrt();
        if len > 1.0 {
            forward /= len;
            right /= len;
        }
        out.forward = PlayerInput::quantize_axis(forward);
        out.right = PlayerInput::quantize_axis(right);
        let mut buttons = latched;
        for (action, button) in BUTTON_ACTIONS {
            if bindings.is_down(input, action) {
                buttons.set(button, true);
            }
        }
        out.buttons = buttons;
        out
    }
}
