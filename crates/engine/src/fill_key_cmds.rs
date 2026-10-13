//! Photoshop's fill keys (#249): ⌥⌫ fills the selection (or the whole layer) with the foreground
//! colour, ⌘⌫ with the background colour, and with ⇧ added they preserve transparency. ⌘⌥⌫ fills
//! from history: the History Brush's source state (the oldest state still held). Photoshop
//! has no menu item for them; here they are ordinary commands with default shortcuts, so they
//! appear in Edit › Keyboard Shortcuts (Tools) and can be rebound, recorded and driven by agents.
//!
//! Each one is `edit.fill` with the colour and Preserve Transparency fixed: one "Fill" history
//! step, the Channels panel target (alpha channel, Quick Mask) honoured, locked layers refused.
//! When targeting a vector shape layer, the fill keys recolor the shape's fill.

use photocraft_doc::{LayerContent, LayerId};
use serde_json::{Map, Value, json};

use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};

/// The fill-key command ids (routed to the Channels panel target like `edit.fill`).
pub const IDS: [&str; 5] = ["edit.fillForeground", "edit.fillBackground", "edit.fillForegroundPreserve", "edit.fillBackgroundPreserve", "edit.fillHistory"];

const PARAMS: &str = r##"{"layer":id?,"target":"pixels"|"mask"|"quickMask"|{"channel":i}?}"##;

fn target_shape_layer(s: &Session, p: &Value) -> Option<LayerId> {
    if let Some(t) = p.get("target")
        && (t.as_str() == Some("mask") || t.as_str() == Some("quickMask") || t.get("channel").is_some())
    {
        return None;
    }
    let st = s.active()?;
    if st.channel_view.target == crate::channel_cmds::ChannelTarget::LayerMask {
        return None;
    }
    let id = if let Some(v) = p.get("layer") { LayerId(v.as_u64()?) } else { st.active_layer? };
    let layer = st.doc.layer(id)?;
    if matches!(layer.content, LayerContent::Shape(_)) { Some(id) } else { None }
}

/// The colour fill keys also recolor a targeted shape layer's fill.
fn enabled(s: &Session) -> std::result::Result<(), String> {
    if target_shape_layer(s, &Value::Null).is_some() {
        return Ok(());
    }
    fill_enabled(s)
}

/// Enabled when `edit.fill` is (Fill from History has no shape-layer meaning).
fn fill_enabled(s: &Session) -> std::result::Result<(), String> {
    let edit_fill = crate::commands::find("edit.fill").ok_or_else(|| "Fill is unavailable".to_string())?;
    (edit_fill.enabled)(s)
}

/// Run `edit.fill` with `color` (the foreground or background colour) and Preserve Transparency,
/// or recolor the vector shape fill when targeting a vector shape layer.
fn fill(s: &mut Session, p: &Value, cmd: &str, background: bool, preserve: bool) -> Result<Value> {
    let mut q: Map<String, Value> = match p {
        Value::Object(m) => m.clone(),
        Value::Null => Map::new(),
        _ => return Err(EngineError::BadParams { cmd: cmd.into(), msg: "expected an object".into() }),
    };

    if let Some(id) = target_shape_layer(s, p) {
        let c = if background { s.tools.background } else { s.tools.foreground };
        let q_u8 = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        let hex = if c[3] < 1.0 {
            format!("#{:02x}{:02x}{:02x}{:02x}", q_u8(c[0]), q_u8(c[1]), q_u8(c[2]), q_u8(c[3]))
        } else {
            format!("#{:02x}{:02x}{:02x}", q_u8(c[0]), q_u8(c[1]), q_u8(c[2]))
        };
        let shape_edit = crate::commands::find("shape.edit").ok_or_else(|| EngineError::Other("shape.edit is unavailable".into()))?;
        return (shape_edit.run)(s, &json!({ "layer": id.0, "fill": hex }));
    }

    // The colour and transparency handling are what the key means; don't let params override them.
    for k in ["color", "contents", "pattern", "preserveTransparency", "opacity"] {
        q.remove(k);
    }
    let c = if background { s.tools.background } else { s.tools.foreground };
    q.insert("color".into(), json!([c[0], c[1], c[2], 1.0]));
    q.insert("preserveTransparency".into(), json!(preserve));
    let edit_fill = crate::commands::find("edit.fill").ok_or_else(|| EngineError::Other("Fill is unavailable".into()))?;
    (edit_fill.run)(s, &Value::Object(q))
}

/// ⌘⌥⌫: `edit.fill` from history, the History Brush's source unless `state` names another.
fn fill_history(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "edit.fillHistory";
    let mut q: Map<String, Value> = match p {
        Value::Object(m) => m.clone(),
        Value::Null => Map::new(),
        _ => return Err(EngineError::BadParams { cmd: CMD.into(), msg: "expected an object".into() }),
    };
    for k in ["color", "contents", "pattern"] {
        q.remove(k);
    }
    q.insert("contents".into(), json!("history"));
    let edit_fill = crate::commands::find("edit.fill").ok_or_else(|| EngineError::Other("Fill is unavailable".into()))?;
    (edit_fill.run)(s, &Value::Object(q))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "edit.fillForeground",
            label: "Fill with Foreground Color",
            menu: &[],
            shortcut: Some("Alt+Backspace"),
            params: PARAMS,
            enabled,
            run: |s, p| fill(s, p, "edit.fillForeground", false, false),
            journal: true,
        },
        CommandSpec {
            id: "edit.fillBackground",
            label: "Fill with Background Color",
            menu: &[],
            shortcut: Some("Cmd+Backspace"),
            params: PARAMS,
            enabled,
            run: |s, p| fill(s, p, "edit.fillBackground", true, false),
            journal: true,
        },
        CommandSpec {
            id: "edit.fillForegroundPreserve",
            label: "Fill with Foreground Color, Preserve Transparency",
            menu: &[],
            shortcut: Some("Alt+Shift+Backspace"),
            params: PARAMS,
            enabled,
            run: |s, p| fill(s, p, "edit.fillForegroundPreserve", false, true),
            journal: true,
        },
        CommandSpec {
            id: "edit.fillBackgroundPreserve",
            label: "Fill with Background Color, Preserve Transparency",
            menu: &[],
            shortcut: Some("Cmd+Shift+Backspace"),
            params: PARAMS,
            enabled,
            run: |s, p| fill(s, p, "edit.fillBackgroundPreserve", true, true),
            journal: true,
        },
        CommandSpec {
            id: "edit.fillHistory",
            label: "Fill from History",
            menu: &[],
            shortcut: Some("Cmd+Alt+Backspace"),
            params: r##"{"state":n? (default: the History Brush's source, the oldest state still held),"layer":id?,"target":"pixels"|"mask"|"quickMask"|{"channel":i}?,"preserveTransparency":bool?}"##,
            enabled: fill_enabled,
            run: fill_history,
            journal: true,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session() -> Session {
        session_at(8)
    }

    fn session_at(depth: u32) -> Session {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 8, "height": 6, "background": "transparent", "depth": depth})).unwrap();
        s.execute("tools.setColors", json!({"foreground": "#ff0000", "background": "#0000ff"})).unwrap();
        s
    }

    fn px(s: &Session, x: i32, y: i32) -> [f32; 4] {
        let st = s.active().unwrap();
        let l = st.doc.layers.last().unwrap();
        let surf = l.surface().unwrap();
        let fmt = surf.format();
        let v = surf.read_region(photocraft_geom::Rect::new(x, y, x + 1, y + 1));
        photocraft_raster::to_rgba(&fmt, &v)
    }

    #[test]
    fn foreground_fills_the_layer_without_a_selection() {
        let mut s = session();
        s.execute("edit.fillForeground", json!({})).unwrap();
        assert_eq!(px(&s, 0, 0), [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(px(&s, 7, 5), [1.0, 0.0, 0.0, 1.0]);
        // One undoable "Fill" step.
        assert!(s.execute("edit.undo", json!({})).is_ok());
        assert_eq!(px(&s, 0, 0)[3], 0.0);
    }

    #[test]
    fn fills_at_every_bit_depth() {
        for depth in [8, 16, 32] {
            let mut s = session_at(depth);
            s.execute("select.rect", json!({"x": 0, "y": 0, "width": 4, "height": 6})).unwrap();
            s.execute("edit.fillForeground", json!({})).unwrap();
            s.execute("select.deselect", json!({})).unwrap();
            s.execute("edit.fillBackgroundPreserve", json!({})).unwrap();
            let [r, g, b, a] = px(&s, 1, 1);
            assert!(r.abs() < 1e-3 && g.abs() < 1e-3 && (b - 1.0).abs() < 1e-3 && (a - 1.0).abs() < 1e-3, "{depth}-bit: {:?}", px(&s, 1, 1));
            assert_eq!(px(&s, 6, 1)[3], 0.0, "{depth}-bit");
        }
    }

    #[test]
    fn fill_from_history_restores_the_history_brush_source_in_the_selection() {
        let mut s = session();
        assert_eq!(crate::commands::find("edit.fillHistory").and_then(|c| c.shortcut), Some("Cmd+Alt+Backspace"));
        s.execute("edit.fillForeground", json!({})).unwrap();
        s.execute("select.rect", json!({"x": 0, "y": 0, "width": 4, "height": 6})).unwrap();
        s.execute("edit.fillHistory", json!({})).unwrap();
        // The oldest state (the new, transparent layer) comes back inside the selection only.
        assert_eq!(px(&s, 1, 1)[3], 0.0);
        assert_eq!(px(&s, 6, 1), [1.0, 0.0, 0.0, 1.0]);
        // A named state, and refused ones change nothing.
        s.execute("select.deselect", json!({})).unwrap();
        let steps = s.active().unwrap().history.past_len();
        for bad in [json!({"state": 99}), json!({"state": "x"}), json!(7)] {
            assert!(s.execute("edit.fillHistory", bad.clone()).is_err(), "{bad}");
        }
        assert_eq!(s.active().unwrap().history.past_len(), steps);
        s.execute("edit.fillHistory", json!({"state": 1})).unwrap();
        assert_eq!(px(&s, 1, 1), [1.0, 0.0, 0.0, 1.0], "state 1 is after the foreground fill");
    }

    #[test]
    fn background_fills_only_the_selection() {
        let mut s = session();
        s.execute("select.rect", json!({"x": 2, "y": 1, "width": 3, "height": 2})).unwrap();
        s.execute("edit.fillBackground", json!({})).unwrap();
        assert_eq!(px(&s, 3, 2), [0.0, 0.0, 1.0, 1.0]);
        assert_eq!(px(&s, 0, 0)[3], 0.0);
        assert_eq!(px(&s, 5, 1)[3], 0.0);
    }

    #[test]
    fn preserve_transparency_recolours_only_existing_pixels() {
        let mut s = session();
        s.execute("select.rect", json!({"x": 0, "y": 0, "width": 2, "height": 2})).unwrap();
        s.execute("edit.fillBackground", json!({})).unwrap();
        s.execute("select.deselect", json!({})).unwrap();
        s.execute("edit.fillForegroundPreserve", json!({})).unwrap();
        assert_eq!(px(&s, 1, 1), [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(px(&s, 4, 4)[3], 0.0, "transparent pixels stay transparent");
        s.execute("edit.fillBackgroundPreserve", json!({})).unwrap();
        assert_eq!(px(&s, 0, 0), [0.0, 0.0, 1.0, 1.0]);
        assert_eq!(px(&s, 4, 4)[3], 0.0);
        // Without ⇧ the whole layer fills.
        s.execute("edit.fillForeground", json!({})).unwrap();
        assert_eq!(px(&s, 4, 4), [1.0, 0.0, 0.0, 1.0]);
    }

    #[test]
    fn params_cannot_change_the_colour_and_bad_params_fail_gracefully() {
        let mut s = session();
        s.execute("edit.fillForeground", json!({"color": "#00ff00", "opacity": 10})).unwrap();
        assert_eq!(px(&s, 0, 0), [1.0, 0.0, 0.0, 1.0]);
        for id in IDS {
            assert!(s.execute(id, json!([1, 2])).is_err(), "{id}");
            assert!(s.execute(id, json!("x")).is_err(), "{id}");
            assert!(s.execute(id, json!({"layer": 99_999})).is_err(), "{id}");
            assert!(s.execute(id, json!({"target": {"channel": 42}})).is_err(), "{id}");
        }
    }

    #[test]
    fn locked_layer_and_no_document_are_errors() {
        let mut s = Session::new();
        for id in IDS {
            assert!(s.execute(id, json!({})).is_err(), "{id} without a document");
        }
        let mut s = session();
        s.execute("layer.lockLayers", json!({"pixels": true})).unwrap();
        for id in IDS {
            let e = s.execute(id, json!({})).unwrap_err().to_string();
            assert!(e.contains("locked"), "{id}: {e}");
        }
        assert_eq!(px(&s, 0, 0)[3], 0.0);
    }

    /// `cargo test --release -p photocraft-engine --lib fill_keys_24mp -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn fill_keys_24mp() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 6000, "height": 4000})).unwrap();
        for (id, sel) in [("edit.fillForeground", false), ("edit.fillBackgroundPreserve", false), ("edit.fillForeground", true)] {
            if sel {
                s.execute("select.rect", json!({"x": 500, "y": 500, "width": 4000, "height": 3000})).unwrap();
            }
            let t = std::time::Instant::now();
            s.execute(id, json!({})).unwrap();
            println!("{id} 24 MP (selection: {sel}): {:.0} ms", t.elapsed().as_secs_f64() * 1e3);
        }
    }

    /// The inks of the active layer's pixel at (0, 0), exactly as stored.
    fn inks(s: &Session) -> Vec<f32> {
        let st = s.active().unwrap();
        let surf = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap();
        surf.read_region(photocraft_geom::Rect::new(0, 0, 1, 1)).into_iter().take(4).collect()
    }

    /// #4196: a foreground picked as CMYK 100/0/0/0 fills a CMYK document with those inks, not with
    /// whatever its sRGB stand-in converts back to (100/28/0/17).
    #[test]
    fn cmyk_pick_fills_a_cmyk_document_with_the_picked_inks() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 4, "height": 4})).unwrap();
        s.execute("image.mode.cmyk", json!({})).unwrap();
        let rgb = photocraft_color::convert::cmyk_to_rgb([1.0, 0.0, 0.0, 0.0]).map(|c| c.clamp(0.0, 1.0));
        s.execute("tools.setColors", json!({"foreground": rgb, "foregroundCmyk": [1.0, 0.0, 0.0, 0.0]})).unwrap();
        s.execute("edit.fillForeground", json!({})).unwrap();
        let got = inks(&s);
        assert!(got.iter().zip([1.0, 0.0, 0.0, 0.0]).all(|(a, b)| (a - b).abs() < 0.01), "{got:?}");
    }

    #[test]
    fn malformed_cmyk_pick_is_an_error_not_a_panic() {
        let mut s = session();
        for bad in [json!([1.0]), json!("cyan"), json!([1.0, 0.0, 0.0, "x"]), json!({"c": 1})] {
            assert!(s.execute("tools.setColors", json!({"foregroundCmyk": bad})).is_err());
        }
    }

    #[test]
    fn registered_with_photoshop_default_shortcuts() {
        let sc = |id| crate::commands::find(id).and_then(|c| c.shortcut);
        assert_eq!(sc("edit.fillForeground"), Some("Alt+Backspace"));
        assert_eq!(sc("edit.fillBackground"), Some("Cmd+Backspace"));
        assert_eq!(sc("edit.fillForegroundPreserve"), Some("Alt+Shift+Backspace"));
        assert_eq!(sc("edit.fillBackgroundPreserve"), Some("Cmd+Shift+Backspace"));
    }

    #[test]
    fn fill_keys_recolor_shape_layer() {
        let mut s = session();
        s.execute("shape.create", json!({"kind": "rect", "rect": [1, 1, 5, 5], "fill": "#000000"})).unwrap();
        let info = s.execute("shape.info", json!({})).unwrap();
        assert_eq!(info["fill"], "#000000");

        // Alt+Backspace fills with foreground (#ff0000)
        s.execute("edit.fillForeground", json!({})).unwrap();
        let info_fg = s.execute("shape.info", json!({})).unwrap();
        assert_eq!(info_fg["fill"], "#ff0000");

        // Cmd+Backspace fills with background (#0000ff)
        s.execute("edit.fillBackground", json!({})).unwrap();
        let info_bg = s.execute("shape.info", json!({})).unwrap();
        assert_eq!(info_bg["fill"], "#0000ff");

        // Undo restores previous fill (#ff0000)
        assert!(s.undo());
        let info_undone = s.execute("shape.info", json!({})).unwrap();
        assert_eq!(info_undone["fill"], "#ff0000");

        // Alt+Shift+Backspace also recolors shape
        s.execute("tools.setColors", json!({"foreground": "#00ff00"})).unwrap();
        s.execute("edit.fillForegroundPreserve", json!({})).unwrap();
        let info_shift = s.execute("shape.info", json!({})).unwrap();
        assert_eq!(info_shift["fill"], "#00ff00");

        // Locked shape layer errors gracefully without crashing
        s.execute("layer.lockLayers", json!({"pixels": true})).unwrap();
        for id in &IDS[..4] {
            let err = s.execute(id, json!({})).unwrap_err().to_string();
            assert!(err.contains("locked"), "{id}: {err}");
        }
    }
}
