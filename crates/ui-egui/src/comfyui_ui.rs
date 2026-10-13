//! Native ComfyUI window. All generation and discovery go through engine commands.
use crate::PhotocraftApp;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Tab {
    #[default]
    Generate,
    Setup,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct State {
    pub open: bool,
    pub tab: Tab,
    pub endpoint: String,
    pub mode: String,
    pub preset: String,
    pub model_files: BTreeMap<String, BTreeMap<String, String>>,
    pub prompt: String,
    pub negative: String,
    pub checkpoint: String,
    pub seed: u64,
    pub steps: u32,
    pub cfg: f64,
    pub denoise: f64,
    pub width: u32,
    pub height: u32,
    pub context_margin: u32,
    pub max_size: u32,
    #[serde(skip)]
    pub region_key: String,
    #[serde(skip)]
    pub region_preview: String,
    pub workflow: String,
    pub bindings: String,
    pub status: String,
    #[serde(skip)]
    pub check_job: Option<u64>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            open: false,
            tab: Tab::Generate,
            endpoint: "http://127.0.0.1:8188".into(),
            mode: "text2image".into(),
            preset: "sd".into(),
            model_files: BTreeMap::new(),
            prompt: String::new(),
            negative: "text, watermark".into(),
            checkpoint: String::new(),
            seed: 0,
            steps: 20,
            cfg: 8.0,
            denoise: 0.65,
            width: 512,
            height: 512,
            context_margin: 128,
            max_size: 2048,
            region_key: String::new(),
            region_preview: String::new(),
            workflow: String::new(),
            bindings: "{}".into(),
            status: String::new(),
            check_job: None,
        }
    }
}

impl State {
    pub(crate) fn select_preset(&mut self, id: &str) -> Result<(), String> {
        let preset = photocraft_plugins::comfyui::presets::get(id).map_err(|e| e.to_string())?;
        self.preset = id.into();
        if !preset.supports(&self.mode) {
            self.mode = if preset.supports("img2img") { "img2img" } else { "inpaint" }.into();
        }
        self.steps = preset.steps;
        self.cfg = preset.cfg;
        self.width = preset.size;
        self.height = preset.size;
        self.negative.clear();
        Ok(())
    }

    fn files_valid(&self) -> bool {
        self.model_files.len() <= photocraft_plugins::comfyui::presets::CATALOG.len()
            && self.model_files.iter().all(|(id, fields)| {
                photocraft_plugins::comfyui::presets::get(id).is_ok()
                    && fields.len() <= 6
                    && fields
                        .iter()
                        .all(|(key, name)| matches!(key.as_str(), "checkpoint" | "unet" | "clip" | "vae" | "lora" | "controlnet") && name.len() <= 16384)
            })
    }

    fn params(&self) -> Result<Value, String> {
        if !self.files_valid() {
            return Err("Invalid model filename settings".into());
        }
        let mut p = json!({"endpoint":self.endpoint,"mode":self.mode,"preset":self.preset,"prompt":self.prompt,"negative":self.negative,"seed":self.seed,"steps":self.steps,"cfg":self.cfg,
            "width":self.width,"height":self.height,"context":self.context_margin,"maxSize":self.max_size});
        if self.mode == "img2img" {
            p["denoise"] = json!(self.denoise);
        }
        if !self.checkpoint.trim().is_empty()
            && (!self.workflow.trim().is_empty() || (self.preset == "sd" && self.model_files.get("sd").and_then(|files| files.get("checkpoint")).is_none()))
        {
            p["checkpoint"] = json!(self.checkpoint.trim());
        }
        if self.workflow.trim().is_empty()
            && let Some(files) = self.model_files.get(&self.preset)
        {
            for (key, filename) in files {
                if !filename.trim().is_empty() {
                    p[key] = json!(filename.trim());
                }
            }
        }
        if !self.workflow.trim().is_empty() {
            if self.workflow.len() > photocraft_plugins::comfyui::MAX_GRAPH_BYTES || self.bindings.len() > 65536 {
                return Err("Workflow or bindings JSON is too large".into());
            }
            p["workflow"] = serde_json::from_str(&self.workflow).map_err(|e| format!("Workflow JSON: {e}"))?;
            p["bindings"] = serde_json::from_str(&self.bindings).map_err(|e| format!("Bindings JSON: {e}"))?;
        }
        Ok(p)
    }
}

const SETUP_PREF: &str = "window.comfyui";

/// Restore only setup; prompts, job status and open windows belong to the current session.
pub(crate) fn restore(app: &mut PhotocraftApp) {
    let Some(saved) = app.session.prefs().dialogs.get(SETUP_PREF).cloned() else { return };
    if let Some(id) = saved.get("preset").and_then(Value::as_str) {
        let _ = app.ui.comfyui.select_preset(id);
    }
    if let Some(files) = saved.get("model_files")
        && let Ok(files) = serde_json::from_value(files.clone())
    {
        let candidate = State { model_files: files, ..Default::default() };
        if candidate.files_valid() {
            app.ui.comfyui.model_files = candidate.model_files;
        }
    }
    if let Some(mode) = saved.get("mode").and_then(Value::as_str).filter(|mode| matches!(*mode, "text2image" | "img2img" | "inpaint")) {
        mode.clone_into(&mut app.ui.comfyui.mode);
    }
    if let Some(value) = saved.get("context").and_then(Value::as_u64).filter(|v| *v <= 1024) {
        app.ui.comfyui.context_margin = value as u32;
    }
    if let Some(value) = saved.get("maxSize").and_then(Value::as_u64).filter(|v| (1024..=2048).contains(v)) {
        app.ui.comfyui.max_size = value as u32;
    }
    for (key, destination, limit) in [
        ("endpoint", &mut app.ui.comfyui.endpoint, 16384),
        ("checkpoint", &mut app.ui.comfyui.checkpoint, 16384),
        ("workflow", &mut app.ui.comfyui.workflow, photocraft_plugins::comfyui::MAX_GRAPH_BYTES),
        ("bindings", &mut app.ui.comfyui.bindings, 65536),
    ] {
        if let Some(text) = saved.get(key).and_then(Value::as_str).filter(|text| text.len() <= limit) {
            text.clone_into(destination);
        }
    }
}

/// Use the existing preference save/retry lifecycle, without copying unchanged graphs per frame.
pub(crate) fn remember(app: &mut PhotocraftApp) -> bool {
    let state = &app.ui.comfyui;
    let fields = [
        ("mode", &state.mode, 16),
        ("preset", &state.preset, 64),
        ("endpoint", &state.endpoint, 16384),
        ("checkpoint", &state.checkpoint, 16384),
        ("workflow", &state.workflow, photocraft_plugins::comfyui::MAX_GRAPH_BYTES),
        ("bindings", &state.bindings, 65536),
    ];
    if !matches!(state.mode.as_str(), "text2image" | "img2img" | "inpaint")
        || fields.iter().any(|(_, text, limit)| text.len() > *limit)
        || photocraft_plugins::comfyui::presets::get(&state.preset).is_err()
        || !state.files_valid()
        || state.context_margin > 1024
        || !(1024..=2048).contains(&state.max_size)
    {
        return false;
    }
    if let Some(saved) = app.session.prefs().dialogs.get(SETUP_PREF)
        && fields.iter().all(|(key, text, _)| saved.get(*key).and_then(Value::as_str) == Some(text.as_str()))
        && saved.get("model_files") == Some(&json!(state.model_files))
        && saved.get("context").and_then(Value::as_u64) == Some(u64::from(state.context_margin))
        && saved.get("maxSize").and_then(Value::as_u64) == Some(u64::from(state.max_size))
    {
        return false;
    }
    let saved = json!({"mode":state.mode,"preset":state.preset,"model_files":state.model_files,"endpoint":state.endpoint,"checkpoint":state.checkpoint,"workflow":state.workflow,"bindings":state.bindings,"context":state.context_margin,"maxSize":state.max_size});
    app.session.prefs.edit(|prefs| prefs.dialogs.insert(SETUP_PREF.into(), saved));
    true
}

fn status(value: &Value) -> String {
    if let Some(ready) = value.get("ready").and_then(Value::as_bool) {
        if ready {
            return "Workflow nodes and model choices are available. Ready to run.".into();
        }
        let mut missing = Vec::new();
        for item in value.get("missingNodes").and_then(Value::as_array).into_iter().flatten() {
            missing.push(format!("Node: {}", item.get("class").and_then(Value::as_str).unwrap_or("unknown")));
        }
        for item in value.get("missingInputs").and_then(Value::as_array).into_iter().flatten() {
            missing.push(format!("{}: {}", item.get("input").and_then(Value::as_str).unwrap_or("input"), item.get("value").unwrap_or(&Value::Null)));
        }
        return format!("Missing dependencies:\n{}\nOpen the setup workflow in ComfyUI and install them there, then check again.", missing.join("\n"));
    }
    if value.get("job").is_some() {
        return "Running in the background. Progress and Cancel are shown in this window.".into();
    }
    if value.get("promptId").is_some() {
        return "ComfyUI finished. Results are available in PhotoCraft.".into();
    }
    if let Some(path) = value.get("path").and_then(Value::as_str) {
        return format!("Saved {path}. Open it in ComfyUI to install missing nodes and models.");
    }
    if value.get("loaded").is_some() {
        return "API workflow loaded. Set its node input bindings before running.".into();
    }
    String::new()
}

pub fn menu(app: &mut PhotocraftApp, id: &str) -> Option<Result<Value, String>> {
    if id != "window.comfyui" {
        return None;
    }
    app.ui.comfyui.open = !app.ui.comfyui.open;
    Some(Ok(json!({"open":app.ui.comfyui.open})))
}

pub(crate) fn handles_job(command: &str) -> bool {
    matches!(command, "comfyui.run" | "comfyui.check")
}

pub fn show(app: &mut PhotocraftApp, ctx: &egui::Context) {
    if !app.ui.comfyui.open {
        return;
    }
    // Also display jobs started by the control channel or reopened after closing this window.
    let running_job = app.session.jobs().into_iter().find(|job| handles_job(&job.command));
    if let Some(job) = &running_job {
        app.ui.comfyui.check_job = Some(job.id.0);
    }
    if let Some(id) = app.ui.comfyui.check_job
        && let Some(job) = app.session.jobs_with_recent().iter().find(|job| job.id.0 == id)
        && job.state != "running"
    {
        app.ui.comfyui.status = job.error.clone().unwrap_or_else(|| job.result.as_ref().map(status).unwrap_or_else(|| job.state.into()));
        app.ui.comfyui.check_job = None;
    }
    let key = format!(
        "{:?}:{}:{}:{}:{}",
        app.session.active().map(|d| (d.doc.id, d.revision)),
        app.ui.comfyui.preset,
        app.ui.comfyui.mode,
        app.ui.comfyui.context_margin,
        app.ui.comfyui.max_size
    );
    if key != app.ui.comfyui.region_key {
        app.ui.comfyui.region_key = key;
        app.ui.comfyui.region_preview = if app.ui.comfyui.mode == "text2image" {
            String::new()
        } else {
            match app
                .session
                .execute("comfyui.region", json!({"preset":app.ui.comfyui.preset,"context":app.ui.comfyui.context_margin,"maxSize":app.ui.comfyui.max_size}))
            {
                Ok(v) => format!(
                    "{}x{} -> {}x{}",
                    v["crop"]["x1"].as_i64().unwrap_or(0) - v["crop"]["x0"].as_i64().unwrap_or(0),
                    v["crop"]["y1"].as_i64().unwrap_or(0) - v["crop"]["y0"].as_i64().unwrap_or(0),
                    v["width"],
                    v["height"]
                ),
                Err(e) => e.to_string(),
            }
        };
    }
    let mut open = true;
    let mut action = None;
    let mut cancel_job = None;
    egui::Window::new(tl!("ComfyUI Generate / Edit")).open(&mut open).default_width(440.0).default_height(660.0).vscroll(true).show(ctx, |ui| {
        let state = &mut app.ui.comfyui;
        ui.horizontal(|ui| {
            ui.selectable_value(&mut state.tab, Tab::Generate, tl!("Generate / Edit"));
            ui.selectable_value(&mut state.tab, Tab::Setup, tl!("Setup"));
        });
        ui.separator();
        if state.tab == Tab::Setup {
            ui.label(tl!("ComfyUI server"));
            ui.text_edit_singleline(&mut state.endpoint);
        }
        let previous = state.preset.clone();
        let selected = photocraft_plugins::comfyui::presets::get(&state.preset).map(|p| p.label).unwrap_or("Unknown model");
        egui::ComboBox::from_label(tl!("Model")).selected_text(selected).show_ui(ui, |ui| {
            for preset in photocraft_plugins::comfyui::presets::CATALOG {
                ui.selectable_value(&mut state.preset, preset.id.into(), preset.label);
            }
        });
        if previous != state.preset {
            let id = state.preset.clone();
            if let Err(error) = state.select_preset(&id) {
                state.status = error;
            }
        }
        let preset = photocraft_plugins::comfyui::presets::get(&state.preset).ok();
        let mode_label = match state.mode.as_str() {
            "img2img" => "Edit image",
            "inpaint" => "Edit selection",
            _ => "Generate from text",
        };
        egui::ComboBox::from_label(tl!("Mode")).selected_text(tl!(mode_label)).show_ui(ui, |ui| {
            for (id, label) in [("text2image", "Generate from text"), ("img2img", "Edit image"), ("inpaint", "Edit selection")] {
                if !state.workflow.trim().is_empty() || preset.is_some_and(|p| p.supports(id)) {
                    ui.selectable_value(&mut state.mode, id.into(), tl!(label));
                }
            }
        });
        if state.tab == Tab::Generate {
            if state.mode == "inpaint" {
                ui.label(tl!("Select the area to edit in PhotoCraft first."));
            } else if state.mode == "img2img" {
                ui.label(tl!("An active selection limits where the edit is applied."));
            }
            ui.label(tl!("Prompt"));
            egui::ScrollArea::vertical().id_salt("comfyui-prompt").max_height(90.0).show(ui, |ui| {
                ui.add(egui::TextEdit::multiline(&mut state.prompt).desired_rows(3).desired_width(f32::INFINITY));
            });
            if !state.preset.starts_with("flux2_") || !state.workflow.trim().is_empty() {
                ui.label(tl!("Negative prompt"));
                ui.text_edit_singleline(&mut state.negative);
            }
            ui.horizontal(|ui| {
                ui.label(tl!("Seed"));
                ui.add(egui::DragValue::new(&mut state.seed).range(0..=9_007_199_254_740_991u64));
                ui.label(tl!("Steps"));
                ui.add(egui::DragValue::new(&mut state.steps).range(1..=150));
            });
            ui.add(egui::Slider::new(&mut state.cfg, 0.0..=30.0).text(tl!("Guidance")));
            if state.mode == "img2img" && (state.preset == "sd" || !state.workflow.trim().is_empty()) {
                ui.add(egui::Slider::new(&mut state.denoise, 0.0..=1.0).text(tl!("Edit strength")));
            }
            if state.mode == "text2image" && state.preset == "hidream_o1" && state.workflow.trim().is_empty() {
                egui::ComboBox::from_label(tl!("Width / Height")).selected_text(format!("{} × {}", state.width, state.height)).show_ui(ui, |ui| {
                    for &(width, height) in photocraft_plugins::comfyui::presets::HIDREAM_SIZES {
                        if ui.selectable_label((state.width, state.height) == (width, height), format!("{width} × {height}")).clicked() {
                            state.width = width;
                            state.height = height;
                        }
                    }
                });
            } else if state.mode == "text2image" {
                ui.horizontal(|ui| {
                    ui.label(tl!("Width / Height"));
                    ui.add(egui::DragValue::new(&mut state.width).range(64..=4096).speed(8.0));
                    ui.add(egui::DragValue::new(&mut state.height).range(64..=4096).speed(8.0));
                });
            }
        }
        if state.tab == Tab::Generate && state.mode != "text2image" {
            ui.label(format!("{}: {}", tl!("Context / processing (padding removed after edit)"), state.region_preview));
        }
        if state.tab == Tab::Setup {
            ui.horizontal(|ui| {
                ui.label(tl!("Surrounding pixels"));
                ui.add(egui::DragValue::new(&mut state.context_margin).range(0..=1024));
            });
            ui.horizontal(|ui| {
                ui.label(tl!("Maximum processing side"));
                ui.add(egui::DragValue::new(&mut state.max_size).range(1024..=2048).speed(16.0));
            });
            if state.workflow.trim().is_empty() {
                if let Some(preset) = preset
                    && let Ok(bundle) = preset.workflow(&state.mode)
                {
                    ui.label(tl!("Model filenames (blank uses preset default)"));
                    for (key, label) in [
                        ("checkpoint", "Checkpoint"),
                        ("unet", "Diffusion model"),
                        ("clip", "Text encoder"),
                        ("vae", "VAE"),
                        ("lora", "LoRA"),
                        ("controlnet", "ControlNet"),
                    ] {
                        if let Some(binding) = bundle.get("bindings").and_then(|v| v.get(key)) {
                            let default = binding
                                .get("node")
                                .and_then(Value::as_str)
                                .zip(binding.get("input").and_then(Value::as_str))
                                .and_then(|(node, input)| bundle.get("api")?.get(node)?.get("inputs")?.get(input)?.as_str())
                                .unwrap_or("");
                            let legacy = if key == "checkpoint" && state.preset == "sd" { state.checkpoint.clone() } else { String::new() };
                            let filename = state.model_files.entry(state.preset.clone()).or_default().entry(key.into()).or_insert(legacy);
                            if state.preset == "sdxl_inpaint" && key == "checkpoint" {
                                ui.label(tl!("Diffusers model folder"));
                            } else {
                                ui.label(tl!(label));
                            }
                            ui.add(egui::TextEdit::singleline(filename).hint_text(default).desired_width(f32::INFINITY));
                        }
                    }
                }
            } else {
                ui.label(tl!("Custom workflow overrides the model preset."));
                ui.label(tl!("Checkpoint (blank uses workflow default)"));
                ui.text_edit_singleline(&mut state.checkpoint);
            }
            ui.collapsing(tl!("Custom model workflow"), |ui| {
                ui.label(tl!("Export API format in ComfyUI. Bind prompt, image and mask to node inputs."));
                if ui.button(tl!("Load API Workflow…")).clicked() {
                    action = Some("load");
                }
                egui::ScrollArea::vertical().id_salt("comfyui-workflow").max_height(120.0).show(ui, |ui| {
                    ui.add(egui::TextEdit::multiline(&mut state.workflow).desired_rows(3).hint_text(tl!("API workflow JSON")));
                });
                egui::ScrollArea::vertical().id_salt("comfyui-bindings").max_height(90.0).show(ui, |ui| {
                    ui.add(egui::TextEdit::multiline(&mut state.bindings).desired_rows(3).hint_text(r#"{"prompt":{"node":"6","input":"text"}}"#));
                });
                ui.hyperlink_to(tl!("Official ComfyUI workflows"), "https://github.com/Comfy-Org/workflow_templates");
            });
            ui.label(tl!("Images are sent as 8-bit sRGB PNG. Results become new layers."));
            if ui.button(tl!("Save Starter Workflow…")).clicked() {
                action = Some("save");
            }
            if ui.add_enabled(!app.session.has_jobs(), egui::Button::new(tl!("Check Availability"))).clicked() {
                action = Some("check");
            }
        }
        if state.tab == Tab::Generate
            && ui
                .add_enabled(
                    !app.session.has_jobs() && (!state.prompt.trim().is_empty() || !state.workflow.trim().is_empty()),
                    egui::Button::new(tl!("Generate / Edit")),
                )
                .clicked()
        {
            action = Some("run");
        }
        if let Some(job) = &running_job {
            if crate::jobs_ui::inline_progress(ui, job) {
                cancel_job = Some(job.id);
            }
        } else if !state.status.is_empty() {
            egui::ScrollArea::vertical().max_height(160.0).show(ui, |ui| {
                ui.label(&state.status);
            });
            if state.tab == Tab::Generate && state.workflow.trim().is_empty() && ui.button(tl!("Save Starter Workflow…")).clicked() {
                action = Some("save");
            }
        }
    });
    app.ui.comfyui.open = open;
    if let Some(id) = cancel_job {
        crate::jobs_ui::cancel(app, id);
    }
    if remember(app) {
        ctx.request_repaint();
    }
    let result = match action {
        Some("load") => Some(app.pick_file_bytes_filtered(&["json"], |app, _, bytes| {
            if bytes.len() > photocraft_plugins::comfyui::MAX_GRAPH_BYTES {
                return Err("Workflow is too large".into());
            }
            let value: Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
            photocraft_plugins::comfyui::validate(&value).map_err(|e| e.to_string())?;
            app.ui.comfyui.workflow = String::from_utf8(bytes).map_err(|e| e.to_string())?;
            Ok(json!({"loaded":true}))
        })),
        Some("save") => {
            let mode = app.ui.comfyui.mode.clone();
            let preset = app.ui.comfyui.preset.clone();
            let params = app.ui.comfyui.params().map(|mut p| {
                if let Some(fields) = p.as_object_mut() {
                    fields.remove("workflow");
                    fields.remove("bindings");
                }
                p
            });
            Some(app.ask_file(crate::FileDialogRequest::Save { suggested: format!("photocraft-comfyui-{preset}-{mode}.json") }, move |app, answer| {
                let crate::FileDialogAnswer::SaveTo(path) = answer else {
                    return Err("Expected a save path".into());
                };
                let mut params = params?;
                params["path"] = json!(path);
                app.run("comfyui.workflow.save", params)
            }))
        }
        Some("check" | "run") => {
            let command = if action == Some("check") { "comfyui.check" } else { "comfyui.run" };
            Some(app.ui.comfyui.params().and_then(|p| app.run(command, p)))
        }
        _ => None,
    };
    if let Some(result) = result {
        match result {
            Ok(v) => {
                if matches!(action, Some("check" | "run")) {
                    app.ui.comfyui.check_job = v.get("job").and_then(Value::as_u64);
                }
                app.ui.comfyui.status = status(&v);
            }
            Err(e) => app.ui.comfyui.status = e,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_dependencies_offer_editable_workflow_from_generate_tab() {
        use egui_kittest::{Harness, kittest::Queryable};
        use std::{cell::RefCell, rc::Rc};
        let requests = Rc::new(RefCell::new(Vec::new()));
        let captured = requests.clone();
        let mut app = PhotocraftApp::new(
            photocraft_engine::Session::new(),
            crate::Services {
                file_dialog: Some(Box::new(move |request, _, reply| {
                    captured.borrow_mut().push(request);
                    reply.send(None);
                })),
                ..Default::default()
            },
        );
        app.ui.comfyui.select_preset("hidream_o1").unwrap();
        app.ui.comfyui.mode = "img2img".into();
        app.ui.comfyui.open = true;
        app.ui.comfyui.status = status(&json!({"ready":false,"missingInputs":[{"input":"ckpt_name","value":"hidream_o1_image_fp8_scaled.safetensors"}]}));
        let mut h = Harness::builder().with_size(egui::vec2(1000.0, 900.0)).build_ui_state(
            |ui, app| {
                show(app, ui.ctx());
                app.poll_file_dialog(ui.ctx(), None);
            },
            app,
        );
        h.run_steps(2);
        assert_eq!(h.state().ui.comfyui.tab, Tab::Generate);
        assert!(h.get_by_label("Save Starter Workflow…").rect().bottom() < 700.0);
        h.get_by_label("Save Starter Workflow…").click();
        h.run_steps(2);
        assert_eq!(*requests.borrow(), vec![crate::FileDialogRequest::Save { suggested: "photocraft-comfyui-hidream_o1-img2img.json".into() }]);
    }

    #[test]
    fn title_bar_button_opens_and_closes_comfyui_without_a_document() {
        use egui_kittest::{Harness, kittest::Queryable};
        for width in [640.0, 1440.0] {
            let mut h = Harness::builder().with_size(egui::vec2(width, 900.0)).build_eframe(|cc| {
                PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
                PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default())
            });
            h.run_steps(2);
            assert!(!h.state().ui.comfyui.open);
            h.get_by_label("ComfyUI").click();
            h.run_steps(2);
            assert!(h.state().ui.comfyui.open);
            h.get_by_label("ComfyUI").click();
            h.run_steps(2);
            assert!(!h.state().ui.comfyui.open);
            assert!(h.state().session.active().is_none());
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn comfyui_progress_stays_in_window_without_blocking_tabs_and_can_cancel() {
        use egui_kittest::{Harness, kittest::Queryable};
        for command in ["comfyui.run", "comfyui.check"] {
            let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
            app.session.execute("file.new", json!({"width":16,"height":16})).unwrap();
            app.ui.comfyui.open = true;
            let (ready_tx, ready_rx) = std::sync::mpsc::channel();
            let (finish_tx, finish_rx) = std::sync::mpsc::channel();
            let started = app
                .session
                .start_job(
                    command,
                    json!({}),
                    "ComfyUI Generate / Edit",
                    true,
                    move |ctx| {
                        ctx.progress(0.4, "Sampling test image");
                        ready_tx.send(()).unwrap();
                        let _ = finish_rx.recv_timeout(std::time::Duration::from_secs(5));
                        ctx.check()
                    },
                    |_, ()| Ok(json!({"promptId":"test"})),
                )
                .unwrap();
            let photocraft_engine::jobs::Started::Job(id) = started else { panic!("expected background job") };
            ready_rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
            // Past the generic dialog's delay: it would normally block these clicks.
            std::thread::sleep(std::time::Duration::from_millis(300));
            let mut h = Harness::builder().with_size(egui::vec2(1000.0, 900.0)).build_ui_state(
                |ui, app| {
                    show(app, ui.ctx());
                    crate::jobs_ui::dialog(app, ui.ctx());
                },
                app,
            );
            h.run_steps(2);
            assert_eq!(h.state().ui.comfyui.check_job, Some(id.0));
            assert!(h.query_by_label("Sampling test image").is_some());
            h.get_by_label("Setup").click();
            h.run_steps(2);
            assert_eq!(h.state().ui.comfyui.tab, Tab::Setup);
            assert!(h.query_by_label("Sampling test image").is_some());
            h.get_by_label("Cancel").click();
            h.run_steps(2);
            assert!(!h.state().session.has_jobs());
            assert!(h.query_by_label("Sampling test image").is_none());
            assert_eq!(h.state().ui.comfyui.status, "cancelled");
            let _ = finish_tx.send(());
            h.state_mut().session.join_cancelled_jobs();
        }
    }

    #[test]
    fn choosing_models_sets_defaults_and_keeps_each_models_setup() {
        let mut state = State::default();
        state.select_preset("qwen_edit_2511").unwrap();
        assert_eq!(state.mode, "img2img");
        assert_eq!(state.steps, 8);
        state.model_files.entry(state.preset.clone()).or_default().insert("unet".into(), "installed-qwen.safetensors".into());
        assert_eq!(state.params().unwrap()["unet"], "installed-qwen.safetensors");
        state.select_preset("flux2_klein_4b").unwrap();
        assert_eq!(state.steps, 4);
        assert_eq!(state.cfg, 1.0);
        assert!(state.params().unwrap().get("unet").is_none());
        state.select_preset("qwen_edit_2511").unwrap();
        assert_eq!(state.params().unwrap()["unet"], "installed-qwen.safetensors");
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.ui.comfyui = state;
        assert!(remember(&mut app));
        app.ui.comfyui = State::default();
        restore(&mut app);
        assert_eq!(app.ui.comfyui.preset, "qwen_edit_2511");
        assert_eq!(app.ui.comfyui.steps, 8);
        assert_eq!(app.ui.comfyui.params().unwrap()["unet"], "installed-qwen.safetensors");
        app.ui.comfyui.model_files.entry("unknown".into()).or_default();
        assert!(app.ui.comfyui.params().is_err());
        assert!(!remember(&mut app));
    }

    #[test]
    fn controlnet_inpaint_keeps_both_model_files_when_switching_and_restarting() {
        use egui_kittest::{Harness, kittest::Queryable};
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.ui.comfyui.select_preset("sdxl_controlnet_inpaint").unwrap();
        assert_eq!(app.ui.comfyui.mode, "inpaint");
        assert_eq!((app.ui.comfyui.steps, app.ui.comfyui.cfg), (15, 8.0));
        app.ui.comfyui.model_files.insert(
            "sdxl_controlnet_inpaint".into(),
            BTreeMap::from([("checkpoint".into(), "base.safetensors".into()), ("controlnet".into(), "dreamer.safetensors".into())]),
        );
        app.ui.comfyui.select_preset("sdxl_inpaint").unwrap();
        assert!(app.ui.comfyui.params().unwrap().get("controlnet").is_none());
        app.ui.comfyui.select_preset("sdxl_controlnet_inpaint").unwrap();
        assert!(remember(&mut app));
        app.ui.comfyui = State::default();
        restore(&mut app);
        let params = app.ui.comfyui.params().unwrap();
        assert_eq!(params["checkpoint"], "base.safetensors");
        assert_eq!(params["controlnet"], "dreamer.safetensors");
        assert_eq!(params["mode"], "inpaint");
        app.ui.comfyui.open = true;
        app.ui.comfyui.tab = Tab::Setup;
        let mut h = Harness::builder().with_size(egui::vec2(1000.0, 900.0)).build_ui_state(|ui, app| show(app, ui.ctx()), app);
        h.run_steps(2);
        assert!(h.query_by_label("ControlNet").is_some());
        assert!(h.query_by_label("Checkpoint").is_some());
        assert!(h.get_by_label("Check Availability").rect().bottom() < 700.0);
        assert!(h.get_by_label("Save Starter Workflow…").rect().bottom() < 700.0);
    }

    #[test]
    fn clearing_model_override_does_not_revive_a_legacy_checkpoint() {
        let mut state = State { checkpoint: "old.safetensors".into(), ..Default::default() };
        assert_eq!(state.params().unwrap()["checkpoint"], "old.safetensors");
        state.model_files.entry("sd".into()).or_default().insert("checkpoint".into(), String::new());
        assert!(state.params().unwrap().get("checkpoint").is_none());
    }

    #[test]
    fn setup_has_its_own_tab_and_keeps_configuration_when_switching() {
        use egui_kittest::{Harness, kittest::Queryable};
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.ui.comfyui.open = true;
        app.ui.comfyui.endpoint = "https://images.example.com/comfy/".into();
        app.ui.comfyui.checkpoint = "installed.safetensors".into();
        let mut h = Harness::builder().with_size(egui::vec2(1000.0, 900.0)).build_ui_state(|ui, app| show(app, ui.ctx()), app);
        h.run_steps(2);
        assert!(h.query_by_label("Prompt").is_some());
        assert!(h.query_by_label("ComfyUI server").is_none());
        assert!(h.query_by_label("Check Availability").is_none());
        h.get_by_label("Setup").click();
        h.run_steps(2);
        assert_eq!(h.state().ui.comfyui.tab, Tab::Setup);
        assert!(h.query_by_label("Prompt").is_none());
        assert!(h.query_by_label("ComfyUI server").is_some());
        assert!(h.query_by_label("Check Availability").is_some());
        assert!(h.get_by_label("Check Availability").rect().bottom() < 650.0);
        assert!(h.get_by_label("Save Starter Workflow…").rect().bottom() < 650.0);
        assert!(h.query_by_label("Custom model workflow").is_some());
        h.get_by_label("Generate / Edit").click();
        h.run_steps(2);
        assert_eq!(h.state().ui.comfyui.tab, Tab::Generate);
        assert!(h.query_by_label("Prompt").is_some());
        assert!(h.query_by_label("ComfyUI server").is_none());
        let p = h.state().ui.comfyui.params().unwrap();
        assert_eq!(p["endpoint"], "https://images.example.com/comfy/");
        assert_eq!(p["checkpoint"], "installed.safetensors");
    }

    #[test]
    fn sdxl_selects_inpaint_and_region_setup_survives_restart() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.ui.comfyui.select_preset("sdxl_inpaint").unwrap();
        assert_eq!(app.ui.comfyui.mode, "inpaint");
        app.ui.comfyui.context_margin = 64;
        app.ui.comfyui.max_size = 1536;
        assert_eq!(app.ui.comfyui.params().unwrap()["context"], 64);
        assert!(remember(&mut app));
        app.ui.comfyui.context_margin = 128;
        app.ui.comfyui.max_size = 2048;
        restore(&mut app);
        assert_eq!((app.ui.comfyui.context_margin, app.ui.comfyui.max_size), (64, 1536));
    }

    #[test]
    fn tab_is_serialized_and_older_settings_open_on_generate() {
        assert_eq!(serde_json::from_value::<State>(json!({})).unwrap().tab, Tab::Generate);
        let setup = State { tab: Tab::Setup, ..Default::default() };
        let saved = serde_json::to_value(&setup).unwrap();
        assert_eq!(saved["tab"], "setup");
        assert_eq!(serde_json::from_value::<State>(saved).unwrap(), setup);
        assert!(serde_json::from_value::<State>(json!({"tab":"unknown"})).is_err());
    }

    #[test]
    fn setup_survives_restart_without_saving_prompts_or_window_state() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.ui.comfyui.endpoint = "https://images.example.com/comfy/".into();
        app.ui.comfyui.mode = "img2img".into();
        app.ui.comfyui.checkpoint = "installed.safetensors".into();
        app.ui.comfyui.workflow = r#"{"1":{"class_type":"Edit","inputs":{}}}"#.into();
        app.ui.comfyui.bindings = "{}".into();
        app.ui.comfyui.prompt = "Private prompt".into();
        app.ui.comfyui.open = true;
        app.ui.comfyui.tab = Tab::Setup;
        assert!(remember(&mut app));
        let revision = app.session.prefs.rev();
        assert!(!remember(&mut app));
        assert_eq!(app.session.prefs.rev(), revision);
        let saved = serde_json::to_string(&app.session.prefs_value()).unwrap();
        assert!(!saved.contains("Private prompt"));
        let mut fresh = PhotocraftApp::new(
            photocraft_engine::Session::new(),
            crate::Services { load_prefs: Some(Box::new(move || Some(saved.clone()))), ..Default::default() },
        );
        assert_eq!(fresh.ui.comfyui.endpoint, app.ui.comfyui.endpoint);
        assert_eq!(fresh.ui.comfyui.mode, app.ui.comfyui.mode);
        assert_eq!(fresh.ui.comfyui.checkpoint, app.ui.comfyui.checkpoint);
        assert_eq!(fresh.ui.comfyui.workflow, app.ui.comfyui.workflow);
        assert_eq!(fresh.ui.comfyui.bindings, app.ui.comfyui.bindings);
        assert_eq!(fresh.ui.comfyui.tab, Tab::Generate);
        assert!(!fresh.ui.comfyui.open);
        assert!(fresh.ui.comfyui.prompt.is_empty());
        fresh.session.prefs.edit(|prefs| {
            prefs.dialogs.insert(SETUP_PREF.into(), json!({"endpoint":false,"workflow":"x".repeat(photocraft_plugins::comfyui::MAX_GRAPH_BYTES + 1)}))
        });
        restore(&mut fresh);
        assert_eq!(fresh.ui.comfyui.endpoint, app.ui.comfyui.endpoint);
        assert_eq!(fresh.ui.comfyui.workflow, app.ui.comfyui.workflow);
    }

    #[test]
    fn window_is_inspectable_and_custom_workflows_keep_bindings() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        menu(&mut app, "window.comfyui").unwrap().unwrap();
        assert!(app.ui.comfyui.open);
        let state = State {
            workflow: r#"{"1":{"class_type":"Edit","inputs":{"prompt":"old"}}}"#.into(),
            bindings: r#"{"prompt":{"node":"1","input":"prompt"}}"#.into(),
            ..Default::default()
        };
        let p = state.params().unwrap();
        assert_eq!(p["bindings"]["prompt"]["node"], "1");
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| show(&mut app, ui.ctx()));
        output.textures_delta.clear();
    }
}
