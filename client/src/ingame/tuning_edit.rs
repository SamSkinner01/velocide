//! Generic editing of flat tuning structs (`MovementTuning`, `NetTuning`) for the debug
//! overlay, and saving edits into `assets/tuning.ron` **without losing its comments**: only the
//! numbers of changed fields are replaced in the file text (missing fields are inserted into
//! their section). The server watches that file, reloads it and broadcasts the new tuning, so
//! client prediction switches to the same values.

use serde::Serialize;
use serde::de::DeserializeOwned;

/// One field of a flat struct, in declaration order.
#[derive(Clone, Debug, PartialEq)]
pub struct Field {
    pub name: String,
    pub value: f64,
    /// Integer field (`u32` etc.): edited in whole steps, written without a decimal point.
    pub integer: bool,
}

/// Fields of a flat struct of numbers (serialised through compact RON to keep the
/// declaration order). Non-numeric fields are skipped.
pub fn fields<T: Serialize>(value: &T) -> Vec<Field> {
    let Ok(text) = ron::to_string(value) else {
        return Vec::new();
    };
    let inner = text.trim().trim_start_matches('(').trim_end_matches(')');
    inner
        .split(',')
        .filter_map(|pair| {
            let (name, raw) = pair.split_once(':')?;
            let raw = raw.trim();
            let value: f64 = raw.parse().ok()?;
            Some(Field {
                name: name.trim().to_string(),
                value,
                integer: !raw.contains(['.', 'e', 'E']),
            })
        })
        .collect()
}

/// RON literal for a field value.
pub fn format_value(value: f64, integer: bool) -> String {
    if integer {
        return format!("{}", value.round().max(0.0) as u64);
    }
    let s = format!("{}", value as f32);
    if s.contains(['.', 'e', 'E', 'N', 'i']) {
        s
    } else {
        format!("{s}.0")
    }
}

/// Rebuilds the struct from fields (fields not listed take their `serde(default)` values).
pub fn from_fields<T: DeserializeOwned>(fields: &[Field]) -> Result<T, String> {
    let body: Vec<String> = fields
        .iter()
        .map(|f| format!("{}:{}", f.name, format_value(f.value, f.integer)))
        .collect();
    ron::from_str(&format!("({})", body.join(","))).map_err(|e| e.to_string())
}

fn is_ident(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// Byte range of the number after `name:` in `text`, if the field is present.
fn find_value(text: &str, name: &str) -> Option<(usize, usize)> {
    let mut search = 0;
    while let Some(off) = text[search..].find(name) {
        let start = search + off;
        let end = start + name.len();
        search = end;
        let before_ok = text[..start]
            .chars()
            .next_back()
            .is_none_or(|c| !is_ident(c));
        let rest = &text[end..];
        let after_name = rest.trim_start();
        if !before_ok || !after_name.starts_with(':') {
            continue;
        }
        // Skip a `//` comment line containing the name.
        let line_start = text[..start].rfind('\n').map_or(0, |i| i + 1);
        if text[line_start..start].contains("//") {
            continue;
        }
        let colon = end + (rest.len() - after_name.len());
        let after_colon = &text[colon + 1..];
        let value_start = colon + 1 + (after_colon.len() - after_colon.trim_start().len());
        let value_len = text[value_start..]
            .find(|c: char| !(c.is_ascii_digit() || "+-.eE".contains(c)))
            .unwrap_or(text.len() - value_start);
        if value_len == 0 {
            continue;
        }
        return Some((value_start, value_start + value_len));
    }
    None
}

/// Writes `changes` (field, value, integer) of `section` (e.g. "movement") into the RON text
/// of a tuning file, keeping everything else (comments, layout) intact.
pub fn rewrite(text: &str, section: &str, changes: &[Field]) -> String {
    let mut out = text.to_string();
    for f in changes {
        let literal = format_value(f.value, f.integer);
        if let Some((a, b)) = find_value(&out, &f.name) {
            out.replace_range(a..b, &literal);
            continue;
        }
        // Not in the file: insert into the section (creating it if needed).
        let line = format!("\n        {}: {},", f.name, literal);
        if let Some((_, b)) = find_section(&out, section) {
            out.insert_str(b, &line);
        } else if let Some(open) = out.find('(') {
            out.insert_str(open + 1, &format!("\n    {section}: ({line}\n    ),"));
        } else {
            out = format!("({section}: ({line}\n),)");
        }
    }
    out
}

/// Position just after `section: (`.
fn find_section(text: &str, section: &str) -> Option<(usize, usize)> {
    let mut search = 0;
    while let Some(off) = text[search..].find(section) {
        let start = search + off;
        let end = start + section.len();
        search = end;
        let rest = text[end..].trim_start();
        if !rest.starts_with(':') {
            continue;
        }
        let after = rest[1..].trim_start();
        if after.starts_with('(') {
            let paren = text.len() - after.len();
            return Some((start, paren + 1));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::tuning::{MovementTuning, NetTuning, Tuning};

    #[test]
    fn movement_round_trips_through_fields() {
        let t = MovementTuning::default();
        let f = fields(&t);
        assert!(f.len() > 40, "all movement values are numbers: {}", f.len());
        assert_eq!(f[0].name, "radius");
        assert!(f.iter().any(|x| x.name == "wall_jumps" && x.integer));
        let back: MovementTuning = from_fields(&f).unwrap();
        assert_eq!(back, t);
        let n = fields(&NetTuning::default());
        let back: NetTuning = from_fields(&n).unwrap();
        assert_eq!(back, NetTuning::default());
    }

    #[test]
    fn rewrite_keeps_comments_and_changes_values() {
        let text = "// header\n(\n    movement: (\n        // jump_speed: 1.0 in a comment\n        jump_speed: 14.0,          // ~2.5 m\n        gravity: 38.0,\n        wall_jumps: 3,\n    ),\n)\n";
        let out = rewrite(
            text,
            "movement",
            &[
                Field {
                    name: "jump_speed".into(),
                    value: 16.5,
                    integer: false,
                },
                Field {
                    name: "wall_jumps".into(),
                    value: 4.0,
                    integer: true,
                },
                Field {
                    name: "dash_speed".into(),
                    value: 50.0,
                    integer: false,
                },
            ],
        );
        assert!(out.contains("// header"));
        assert!(out.contains("// jump_speed: 1.0 in a comment"));
        assert!(out.contains("jump_speed: 16.5,          // ~2.5 m"));
        assert!(out.contains("wall_jumps: 4,"));
        assert!(out.contains("dash_speed: 50.0,"));
        let t: Tuning = ron::from_str(&out).unwrap();
        assert_eq!(t.movement.jump_speed, 16.5);
        assert_eq!(t.movement.wall_jumps, 4);
        assert_eq!(t.movement.dash_speed, 50.0);
        assert_eq!(t.movement.gravity, 38.0);
    }

    #[test]
    fn rewrite_adds_missing_sections() {
        let out = rewrite(
            "(\n    movement: (\n    ),\n)",
            "net",
            &[Field {
                name: "interp_delay_ms".into(),
                value: 70.0,
                integer: false,
            }],
        );
        let t: Tuning = ron::from_str(&out).unwrap();
        assert_eq!(t.net.interp_delay_ms, 70.0);
    }

    #[test]
    fn real_tuning_file_survives_a_rewrite() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../assets/tuning.ron");
        let text = std::fs::read_to_string(path).unwrap();
        let out = rewrite(
            &text,
            "movement",
            &[Field {
                name: "gravity".into(),
                value: 40.0,
                integer: false,
            }],
        );
        let t: Tuning = ron::from_str(&out).unwrap();
        assert_eq!(t.movement.gravity, 40.0);
        assert_eq!(out.lines().count(), text.lines().count());
    }
}
