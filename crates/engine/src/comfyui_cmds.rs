//! ComfyUI generation/edit commands; network work uses the existing cancellable job system.
use crate::{EngineError, Result, Session, commands::CommandSpec};
use photocraft_plugins::comfyui;
use serde_json::{Value, json};
#[path = "comfyui_region.rs"]
mod region;

fn error(e: impl std::fmt::Display) -> EngineError {
    EngineError::Other(e.to_string())
}
fn bad(message: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: "comfyui".into(), msg: message.into() }
}

fn text<'a>(p: &'a Value, key: &str) -> Result<&'a str> {
    p.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty() && s.len() <= 16384)
        .ok_or_else(|| bad(format!("{key} must be a non-empty string (maximum 16384 bytes)")))
}

fn mode(p: &Value) -> Result<&str> {
    let mode = text(p, "mode")?;
    if !matches!(mode, "text2image" | "img2img" | "inpaint") {
        return Err(bad("mode must be text2image, img2img or inpaint"));
    }
    Ok(mode)
}

fn preset(p: &Value) -> Result<&'static comfyui::presets::Preset> {
    let id = match p.get("preset") {
        Some(_) => text(p, "preset")?,
        None => "sd",
    };
    comfyui::presets::get(id).map_err(error)
}

fn number(p: &Value, key: &str, default: f64, min: f64, max: f64, whole: bool) -> Result<f64> {
    let Some(v) = p.get(key) else { return Ok(default) };
    v.as_f64()
        .filter(|n| n.is_finite() && *n >= min && *n <= max && (!whole || n.fract() == 0.0))
        .ok_or_else(|| bad(format!("{key} must be {}in {min}..={max}", if whole { "an integer " } else { "" })))
}

fn binding(p: &Value, key: &str, mode: &str) -> Result<Option<(String, String)>> {
    if p.get("workflow").is_some() {
        let bindings = p.get("bindings").and_then(Value::as_object).ok_or_else(|| bad("custom workflow requires a bindings object"))?;
        return bindings.get(key).map(|b| Ok((text(b, "node")?.to_string(), text(b, "input")?.to_string()))).transpose();
    }
    let bundle = preset(p)?.workflow(mode).map_err(error)?;
    bundle.get("bindings").and_then(|b| b.get(key)).map(|b| Ok((text(b, "node")?.to_string(), text(b, "input")?.to_string()))).transpose()
}

fn patch(graph: &mut Value, p: &Value, key: &str, mode: &str, value: Value) -> Result<()> {
    if let Some((node, input)) = binding(p, key, mode)? {
        comfyui::bind(graph, &node, &input, value.clone()).map_err(error)?;
    }
    if p.get("workflow").is_none()
        && matches!(key, "width" | "height")
        && let Some((node, input)) = binding(p, &format!("{key}_extra"), mode)?
    {
        comfyui::bind(graph, &node, &input, value).map_err(error)?;
    }
    Ok(())
}

fn graph(p: &Value) -> Result<Value> {
    let mode = mode(p)?;
    let preset = preset(p)?;
    let mut graph = match p.get("workflow") {
        Some(v) => {
            comfyui::validate(v).map_err(error)?;
            v.clone()
        }
        None => preset.workflow(mode).map_err(error)?.get("api").cloned().ok_or_else(|| error("Missing preset API graph"))?,
    };
    comfyui::validate(&graph).map_err(error)?;
    for key in ["prompt", "negative", "checkpoint", "unet", "clip", "vae", "lora", "controlnet"] {
        if let Some(value) = p.get(key) {
            let s = value.as_str().filter(|s| s.len() <= 16384).ok_or_else(|| bad(format!("{key} must be a string")))?;
            if matches!(key, "checkpoint" | "unet" | "clip" | "vae" | "lora" | "controlnet") && binding(p, key, mode)?.is_none() {
                return Err(bad(format!("Selected workflow has no {key} model binding")));
            }
            patch(&mut graph, p, key, mode, json!(s))?;
        }
    }
    for (key, default, min, max, whole) in [
        ("seed", 0.0, 0.0, 9_007_199_254_740_991.0, true),
        ("steps", f64::from(preset.steps), 1.0, 150.0, true),
        ("cfg", preset.cfg, 0.0, 30.0, false),
        (
            "denoise",
            if preset.id == "sdxl_inpaint" {
                0.99
            } else if mode == "img2img" {
                0.65
            } else {
                1.0
            },
            0.0,
            1.0,
            false,
        ),
        ("width", f64::from(preset.size), 64.0, 4096.0, true),
        ("height", f64::from(preset.size), 64.0, 4096.0, true),
    ] {
        let n = number(p, key, default, min, max, whole)?;
        if matches!(key, "width" | "height") && n % 8.0 != 0.0 {
            return Err(bad("width and height must be multiples of 8"));
        }
        if p.get("workflow").is_none()
            && matches!(key, "width" | "height")
            && (preset.id.starts_with("flux2_") || preset.id.starts_with("mage_"))
            && n % 16.0 != 0.0
        {
            return Err(bad("FLUX and Mage width and height must be multiples of 16"));
        }
        // Custom workflows keep their defaults unless the caller supplied an override.
        if p.get("workflow").is_none() || p.get(key).is_some() {
            patch(&mut graph, p, key, mode, if whole { json!(n as u64) } else { json!(n) })?;
        }
    }
    if p.get("workflow").is_none() && preset.id == "hidream_o1" {
        let size = (
            p.get("width").and_then(Value::as_u64).unwrap_or(u64::from(preset.size)),
            p.get("height").and_then(Value::as_u64).unwrap_or(u64::from(preset.size)),
        );
        if !comfyui::presets::HIDREAM_SIZES.iter().any(|&(width, height)| size == (u64::from(width), u64::from(height))) {
            return Err(bad("HiDream O1 needs a supported training resolution, such as 2048×2048"));
        }
    }
    for key in ["prompt", "image", "mask"] {
        let required = match key {
            "prompt" => mode == "text2image" || p.get("workflow").is_none(),
            "image" => mode != "text2image",
            "mask" => mode == "inpaint",
            _ => false,
        };
        if required && binding(p, key, mode)?.is_none() {
            return Err(bad(format!("workflow needs a {key} binding")));
        }
    }
    Ok(graph)
}

fn setup(_: &mut Session, p: &Value) -> Result<Value> {
    let mode = mode(p)?;
    let preset = preset(p)?;
    let bundle = preset.workflow(mode).map_err(error)?;
    let mut ui = bundle.get("ui").cloned().ok_or_else(|| error("Missing preset UI graph"))?;
    let api = graph(p)?;
    // Widget indices are explicit for bundled graphs; never guess arbitrary custom ordering.
    for binding in bundle.get("bindings").and_then(Value::as_object).into_iter().flat_map(|b| b.values()) {
        if let Some(id) = binding.get("node").and_then(Value::as_str)
            && let Some(input) = binding.get("input").and_then(Value::as_str)
            && let Some(index) = binding.get("widget").and_then(Value::as_u64).and_then(|i| usize::try_from(i).ok())
            && let Some(value) = api.get(id).and_then(|n| n.get("inputs")).and_then(|i| i.get(input))
            && let Some(nodes) = ui.get_mut("nodes").and_then(Value::as_array_mut)
            && let Some(node) = nodes.iter_mut().find(|n| n.get("id").and_then(Value::as_u64).map(|n| n.to_string()).as_deref() == Some(id))
            && let Some(widget) = node.get_mut("widgets_values").and_then(Value::as_array_mut).and_then(|v| v.get_mut(index))
        {
            *widget = value.clone();
        }
    }
    let catalog: Vec<Value> = comfyui::presets::CATALOG
        .iter()
        .map(|preset| {
            let modes: Vec<_> = ["text2image", "img2img", "inpaint"].into_iter().filter(|mode| preset.supports(mode)).collect();
            json!({"id":preset.id,"label":preset.label,"modes":modes,"steps":preset.steps,"cfg":preset.cfg,"width":preset.size,"height":preset.size})
        })
        .collect();
    Ok(
        json!({"mode":mode,"preset":preset.id,"workflow":ui,"api":api,"bindings":bundle.get("bindings"),"presets":catalog,"source":if preset.id == "sdxl_union_inpaint" { "https://huggingface.co/xinsir/controlnet-union-sdxl-1.0" } else if preset.id == "sdxl_controlnet_inpaint" { "https://huggingface.co/destitech/controlnet-inpaint-dreamer-sdxl" } else if preset.id == "sdxl_inpaint" { "https://github.com/Comfy-Org/ComfyUI/blob/master/nodes.py" } else { comfyui::presets::SOURCE },
        "instructions":"Save workflow as JSON, open it in ComfyUI and install missing nodes/models there. PhotoCraft does not install them."}),
    )
}

fn save(s: &mut Session, p: &Value) -> Result<Value> {
    let reply = setup(s, p)?;
    let bytes = serde_json::to_vec_pretty(reply.get("workflow").ok_or_else(|| error("Missing setup workflow"))?).map_err(error)?;
    let path = text(p, "path")?;
    #[cfg(not(target_arch = "wasm32"))]
    {
        photocraft_format::atomic_write(std::path::Path::new(path), &bytes).map_err(error)?;
        Ok(json!({"path":path}))
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (path, bytes);
        Err(error("Save setup workflow requires native PhotoCraft; comfyui.workflow returns its JSON on the web"))
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn fit_selected_output(doc: &mut photocraft_doc::Document, size: photocraft_doc::Size) -> Result<()> {
    for dimensions in [doc.size, size] {
        if dimensions.width == 0
            || dimensions.height == 0
            || dimensions.width > 8192
            || dimensions.height > 8192
            || u64::from(dimensions.width) * u64::from(dimensions.height) > 16_777_216
        {
            return Err(bad("selected edit dimensions are limited to 16 megapixels and 8192 pixels per side"));
        }
    }
    if doc.size == size {
        return Ok(());
    }
    let bounds = doc.bounds();
    let sx = f64::from(size.width) / f64::from(doc.size.width);
    let sy = f64::from(size.height) / f64::from(doc.size.height);
    let surface = doc.layers.first_mut().and_then(|layer| layer.surface_mut()).ok_or_else(|| error("ComfyUI output has no raster pixels"))?;
    *surface = photocraft_algo::resample::resize_surface_in_canvas(surface, sx, sy, photocraft_algo::resample::Resample::Lanczos, bounds);
    *surface = photocraft_algo::resample::crop_surface(surface, photocraft_geom::Rect::new(0, 0, size.width as i32, size.height as i32));
    doc.size = size;
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
fn check(s: &mut Session, p: &Value) -> Result<Value> {
    let client = comfyui::Client::new(text(p, "endpoint")?).map_err(error)?;
    let graph = graph(p)?;
    crate::jobs::run(s, "Check ComfyUI", false, move |_| comfyui::availability(&graph, &client.object_info().map_err(error)?).map_err(error), |_, v| Ok(v))
}

#[cfg(not(target_arch = "wasm32"))]
fn run(s: &mut Session, p: &Value) -> Result<Value> {
    use photocraft_doc::{Layer, LayerContent, LayerMask};
    let mode = mode(p)?.to_string();
    if p.get("workflow").is_none() || mode == "text2image" || binding(p, "prompt", &mode)?.is_some() {
        text(p, "prompt")?;
    }
    let client = comfyui::Client::new(text(p, "endpoint")?).map_err(error)?;
    let mut graph = graph(p)?;
    let image_binding = binding(p, "image", &mode)?;
    let mask_binding = binding(p, "mask", &mode)?;
    let snapshot = s.active().map(|d| d.doc.clone());
    if mode != "text2image" && snapshot.is_none() {
        return Err(EngineError::NoDocument);
    }
    if mode == "inpaint" && snapshot.as_ref().is_none_or(|d| d.selection.is_none()) {
        return Err(bad("inpaint requires a selection"));
    }
    // Instruction-edit models render a complete image. Restrict its application to
    // the selection captured with the input, including feathered mask values.
    let selection_masked = mode != "text2image" && snapshot.as_ref().is_some_and(|d| d.selection.is_some());
    let region = if mode != "text2image" { Some(region::Region::plan(snapshot.as_ref().ok_or(EngineError::NoDocument)?, p)?) } else { None };
    if let Some(region) = &region {
        patch(&mut graph, p, "width", &mode, json!(region.width))?;
        patch(&mut graph, p, "height", &mode, json!(region.height))?;
    }
    let label = "ComfyUI Generate / Edit";
    let result_mode = mode.clone();
    crate::jobs::run(
        s,
        label,
        snapshot.is_some(),
        move |ctx| {
            ctx.check()?;
            ctx.progress(0.05, "Checking ComfyUI nodes and models");
            let available = comfyui::availability(&graph, &client.object_info().map_err(error)?).map_err(error)?;
            if available.get("ready").and_then(Value::as_bool) != Some(true) {
                return Err(error(format!("ComfyUI workflow is not ready: {available}")));
            }
            if mode != "text2image" {
                let doc = snapshot.as_ref().ok_or(EngineError::NoDocument)?;
                let region = region.as_ref().ok_or_else(|| error("Missing edit region"))?;
                ctx.check()?;
                ctx.progress(
                    0.1,
                    &format!("Uploading {}x{} region (processing {}x{})", region.crop.width(), region.crop.height(), region.width, region.height),
                );
                let (node, input) = image_binding.as_ref().ok_or_else(|| bad("workflow needs an image binding"))?;
                let png = region.input_png(doc, false)?;
                ctx.check()?;
                comfyui::bind(&mut graph, node, input, json!(client.upload("photocraft-input.png", png).map_err(error)?)).map_err(error)?;
                if mode == "inpaint" {
                    ctx.check()?;
                    let (node, input) = mask_binding.as_ref().ok_or_else(|| bad("workflow needs a mask binding"))?;
                    let png = region.input_png(doc, true)?;
                    ctx.check()?;
                    comfyui::bind(&mut graph, node, input, json!(client.upload("photocraft-mask.png", png).map_err(error)?)).map_err(error)?;
                }
            }
            ctx.check()?;
            let id = client.submit(&graph).map_err(error)?;
            ctx.progress(0.2, &format!("ComfyUI queued: {id}"));
            let deadline = std::time::Instant::now();
            let descriptors = loop {
                if ctx.cancelled() {
                    let _ = client.cancel_pending(&id);
                    return Err(EngineError::Cancelled);
                }
                if deadline.elapsed() > std::time::Duration::from_secs(600) {
                    let _ = client.cancel_pending(&id);
                    return Err(error(format!("ComfyUI job {id} timed out after 10 minutes")));
                }
                if let Some(outputs) = comfyui::outputs(&client.history(&id).map_err(error)?, &id).map_err(error)? {
                    break outputs;
                }
                std::thread::sleep(std::time::Duration::from_millis(250));
            };
            ctx.progress(0.85, "Downloading ComfyUI results");
            let mut results = Vec::new();
            let mut total = 0usize;
            let mut total_pixels = 0u64;
            for descriptor in descriptors {
                ctx.check()?;
                let bytes = client.download(&descriptor).map_err(error)?;
                total = total.saturating_add(bytes.len());
                if total > comfyui::MAX_IMAGE_BYTES {
                    return Err(bad("combined ComfyUI outputs exceed 64 MiB"));
                }
                if photocraft_codecs::detect(&bytes) != Some(photocraft_codecs::Format::Png) {
                    return Err(bad("ComfyUI output must be PNG (use SaveImage or PreviewImage)"));
                }
                let image = photocraft_codecs::decode_with(
                    &bytes,
                    &photocraft_codecs::DecodeOptions {
                        limits: photocraft_codecs::Limits { max_width: 8192, max_height: 8192, max_pixels: 16_777_216, max_alloc: 128 * 1024 * 1024 },
                        ..Default::default()
                    },
                )
                .map_err(error)?;
                // Budget the fitted outputs too: small model results may expand to large canvases.
                let pixels = if let Some(region) = &region {
                    u64::from(region.crop.width()) * u64::from(region.crop.height())
                } else {
                    u64::from(image.width()) * u64::from(image.height())
                };
                total_pixels = total_pixels.saturating_add(pixels.max(u64::from(image.width()) * u64::from(image.height())));
                if total_pixels > 67_108_864
                    || snapshot.as_ref().is_some_and(|doc| total_pixels.saturating_mul(doc.pixel_format().bytes_per_pixel() as u64) > 512 * 1024 * 1024)
                {
                    return Err(bad("combined restored ComfyUI outputs exceed 64 megapixels or the 512 MiB native-depth budget"));
                }
                // Preflight PNG dimensions/allocation before the normal import path.
                drop(image);
                let mut imported = photocraft_io::import("comfyui.png", &bytes).map_err(error)?.document;
                if imported.icc_profile.is_none() && imported.mode == photocraft_color::ColorMode::Rgb {
                    imported.icc_profile = Some(photocraft_cms::Builtin::Srgb.profile().to_bytes());
                }
                if u64::from(imported.size.width) * u64::from(imported.size.height) > 16_777_216 {
                    return Err(bad("ComfyUI output exceeds 16 megapixels"));
                }
                if let Some(region) = &region {
                    ctx.check()?;
                    region.restore(&mut imported)?;
                }
                results.push(imported);
            }
            ctx.check()?;
            Ok((id, results, snapshot, region))
        },
        move |s, (id, results, snapshot, region)| {
            if snapshot.is_none() {
                let documents: Vec<_> = results
                    .into_iter()
                    .map(|mut doc| {
                        doc.name = "ComfyUI Generated Image".into();
                        s.add_document(doc, None)
                    })
                    .collect();
                return Ok(json!({"promptId":id,"documents":documents}));
            }
            let doc = snapshot.as_ref().ok_or(EngineError::NoDocument)?;
            let profile = crate::color_cmds::document_profile(doc);
            let mut layers = Vec::new();
            for (i, mut result) in results.into_iter().enumerate() {
                // Standard ComfyUI outputs are sRGB when untagged. Respect an embedded profile.
                crate::color_cmds::convert_document(&mut result, &profile, photocraft_cms::Intent::RelativeColorimetric, true)?;
                let surface =
                    result.layers.first().and_then(|l| l.surface()).ok_or_else(|| error("ComfyUI output has no raster pixels"))?.convert(doc.pixel_format());
                let surface = if let Some(region) = &region { surface.translated(region.crop.x0, region.crop.y0, result.bounds()) } else { surface };
                let mut layer = Layer::new(format!("ComfyUI {result_mode} {}", i + 1), LayerContent::Raster(surface));
                if selection_masked {
                    layer.mask = doc.selection.as_ref().map(|surface| LayerMask { surface: surface.clone(), ..LayerMask::reveal_all() });
                }
                layers.push(layer);
            }
            let ids = s.edit(label, move |doc, active| {
                let mut ids = Vec::new();
                for layer in layers {
                    let id = doc.insert_above(*active, layer);
                    *active = Some(id);
                    ids.push(id.0);
                }
                Ok(ids)
            })?;
            Ok(json!({"promptId":id,"layers":ids,"selectionMasked":selection_masked,"transport":"sRGB 8-bit PNG","region":region}))
        },
    )
}

#[cfg(target_arch = "wasm32")]
fn check(_: &mut Session, p: &Value) -> Result<Value> {
    graph(p)?;
    Err(error("ComfyUI HTTP integration currently requires native PhotoCraft"))
}
#[cfg(target_arch = "wasm32")]
fn run(_: &mut Session, p: &Value) -> Result<Value> {
    graph(p)?;
    Err(error("ComfyUI HTTP integration currently requires native PhotoCraft"))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "comfyui.region",
            label: "Preview ComfyUI Edit Region",
            menu: &[],
            shortcut: None,
            params: r#"{"preset":str="sd","context":0..1024=128,"maxSize":1024..2048=2048} ? crop, inner dimensions and padding"#,
            enabled: crate::commands::always,
            run: |s, p| serde_json::to_value(region::Region::plan(&s.active().ok_or(EngineError::NoDocument)?.doc, p)?).map_err(error),
            journal: false,
        },
        CommandSpec {
            id: "comfyui.workflow",
            label: "ComfyUI Setup Workflow",
            menu: &[],
            shortcut: None,
            params: r#"{"mode":"text2image|img2img|inpaint","preset":str="sd","checkpoint":str?,"unet":str?,"clip":str?,"vae":str?,"lora":str?,"controlnet":str?} → {workflow,api,bindings,presets,instructions}"#,
            enabled: crate::commands::always,
            run: setup,
            journal: false,
        },
        CommandSpec {
            id: "comfyui.workflow.save",
            label: "Save ComfyUI Setup Workflow",
            menu: &[],
            shortcut: None,
            params: r#"{"mode":"text2image|img2img|inpaint","preset":str="sd","path":str,"checkpoint":str?,"unet":str?,"clip":str?,"vae":str?,"lora":str?,"controlnet":str?}"#,
            enabled: crate::commands::always,
            run: save,
            journal: false,
        },
        CommandSpec {
            id: "comfyui.check",
            label: "Check ComfyUI Workflow",
            menu: &[],
            shortcut: None,
            params: r#"{"endpoint":str,"mode":"text2image|img2img|inpaint","preset":str="sd","checkpoint":str?,"unet":str?,"clip":str?,"vae":str?,"lora":str?,"controlnet":str?,"workflow":API graph?,"bindings":object?} → {ready,missingNodes,missingInputs}"#,
            enabled: crate::commands::always,
            run: check,
            journal: false,
        },
        CommandSpec {
            id: "comfyui.run",
            label: "ComfyUI Generate / Edit",
            menu: &[],
            shortcut: None,
            params: r#"{"endpoint":str,"mode":"text2image|img2img|inpaint","preset":str="sd","prompt":str?,"negative":str?,"checkpoint":str?,"unet":str?,"clip":str?,"vae":str?,"lora":str?,"controlnet":str?,"seed":int=0,"steps":1..150?,"cfg":0..30?,"denoise":0..1?,"width":64..4096?,"height":64..4096?,"context":0..1024=128,"maxSize":1024..2048=2048,"workflow":API graph?,"bindings":object?} → {promptId,layers|documents}; numeric defaults depend on preset; prompt required for starters/text generation/prompt binding; edits crop context, pad, restore placement and honor the original selection"#,
            enabled: crate::commands::always,
            run,
            journal: true,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn selected_outputs_fit_non_square_canvases_without_cropping_and_reject_absurd_sizes() {
        use photocraft_doc::{ColorMode, Document, SampleType, Size};
        use photocraft_geom::Rect;
        for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let mut doc = Document::with_background("output", Size::new(8, 8), ColorMode::Rgb, depth, photocraft_color::Color::WHITE);
            let surface = doc.layers.first_mut().unwrap().surface_mut().unwrap();
            surface.fill_rect(Rect::new(0, 0, 8, 8), &[1.0, 0.0, 0.0, 1.0]);
            surface.fill_rect(Rect::new(4, 0, 8, 8), &[0.0, 0.0, 1.0, 1.0]);
            fit_selected_output(&mut doc, Size::new(16, 8)).unwrap();
            assert_eq!(doc.size, Size::new(16, 8));
            let surface = doc.layers.first().unwrap().surface().unwrap();
            assert_eq!(surface.format().sample, depth);
            assert_eq!(surface.pixel(1, 4), vec![1.0, 0.0, 0.0, 1.0]);
            assert_eq!(surface.pixel(14, 4), vec![0.0, 0.0, 1.0, 1.0]);
            assert_eq!(surface.pixel(15, 7)[3], 1.0);
            let before = doc.clone();
            for size in [Size::new(0, 8), Size::new(8, 0), Size::new(u32::MAX, 8), Size::new(8192, 8192)] {
                assert!(fit_selected_output(&mut doc, size).is_err());
                assert_eq!(doc, before);
            }
            doc.size = Size::new(0, 8);
            assert!(fit_selected_output(&mut doc, Size::new(16, 8)).is_err());
        }
    }

    #[test]
    fn union_inpaint_exports_promax_repaint_and_uncomposited_output() {
        let p = json!({"mode":"inpaint","preset":"sdxl_union_inpaint","checkpoint":"base.safetensors","controlnet":"union-promax.safetensors","width":1536,"height":1024});
        let result = setup(&mut Session::new(), &p).unwrap();
        let api = &result["api"];
        assert_eq!(api["23"]["inputs"]["control_net_name"], "union-promax.safetensors");
        assert_eq!(api["26"]["inputs"]["type"], "repaint");
        assert_eq!(api["24"]["inputs"]["control_net"], json!(["26", 0]));
        assert_eq!(api["3"]["inputs"]["negative"], json!(["24", 1]));
        assert_eq!(api["21"]["inputs"]["color"], 0);
        assert_eq!(api["21"]["inputs"]["width"], 1536);
        assert_eq!(api["101"]["inputs"]["value"], 0.0);
        assert_eq!(api["22"]["inputs"]["mask"], json!(["101", 0]));
        assert_eq!(api["25"]["class_type"], "VAEEncodeForInpaint");
        assert_eq!(api["25"]["inputs"]["pixels"], json!(["22", 0]));
        assert_eq!(api["3"]["inputs"]["steps"], 25);
        assert_eq!(api["3"]["inputs"]["cfg"], 4.5);
        assert_eq!(api["9"]["inputs"]["images"], json!(["8", 0]), "keep decoded context for the editable PhotoCraft mask");
        assert_eq!(result["source"], "https://huggingface.co/xinsir/controlnet-union-sdxl-1.0");
        let loader = result["workflow"]["nodes"].as_array().unwrap().iter().find(|n| n["id"] == 23).unwrap();
        assert_eq!(loader["widgets_values"][0], "union-promax.safetensors");
    }

    #[test]
    fn controlnet_inpaint_exports_mask_conditioning_and_both_model_overrides() {
        let p = json!({"mode":"inpaint","preset":"sdxl_controlnet_inpaint","checkpoint":"base.safetensors","controlnet":"dreamer.safetensors","width":1536,"height":1024});
        let result = setup(&mut Session::new(), &p).unwrap();
        let api = &result["api"];
        assert_eq!(api["1"]["inputs"]["ckpt_name"], "base.safetensors");
        assert_eq!(api["23"]["inputs"]["control_net_name"], "dreamer.safetensors");
        assert_eq!(api["22"]["inputs"]["mask"], json!(["100", 0]));
        assert_eq!(api["24"]["inputs"]["image"], json!(["22", 0]));
        assert_eq!(api["21"]["inputs"]["color"], 0xffffff);
        assert_eq!(api["3"]["inputs"]["latent_image"], json!(["25", 0]));
        assert_eq!(api["3"]["inputs"]["denoise"], 1.0);
        for id in [21, 25] {
            assert_eq!(api[id.to_string()]["inputs"]["width"], 1536);
            let node = result["workflow"]["nodes"].as_array().unwrap().iter().find(|n| n["id"] == id).unwrap();
            assert_eq!(node["widgets_values"][0], 1536);
        }
        let loader = result["workflow"]["nodes"].as_array().unwrap().iter().find(|n| n["id"] == 23).unwrap();
        assert_eq!(loader["widgets_values"][0], "dreamer.safetensors");
        assert!(graph(&json!({"mode":"inpaint","preset":"sdxl_controlnet_inpaint","controlnet":42})).is_err());
        assert!(graph(&json!({"mode":"inpaint","preset":"sdxl_inpaint","controlnet":"wrong"})).is_err());
    }

    #[test]
    fn presets_use_model_specific_defaults_and_synchronize_flux_dimensions() {
        let flux = graph(&json!({"mode":"text2image","preset":"flux2_klein_9b","width":768,"height":1152,"unet":"custom-flux.safetensors"})).unwrap();
        assert_eq!(flux["1"]["inputs"]["unet_name"], "custom-flux.safetensors");
        assert_eq!(flux["2"]["inputs"]["clip_name"], "qwen_3_8b_fp8mixed.safetensors");
        assert_eq!(flux["33"]["inputs"]["steps"], 4);
        assert_eq!(flux["34"]["inputs"]["cfg"], 1.0);
        for id in ["30", "33"] {
            assert_eq!(flux[id]["inputs"]["width"], 768);
            assert_eq!(flux[id]["inputs"]["height"], 1152);
        }
        let qwen = graph(&json!({"mode":"img2img","preset":"qwen_edit_2511_gguf"})).unwrap();
        assert_eq!(qwen["1"]["class_type"], "UnetLoaderGGUF");
        assert_eq!(qwen["35"]["inputs"]["steps"], 8);
        assert_eq!(qwen["35"]["inputs"]["denoise"], 1);
        let hd = graph(&json!({"mode":"img2img","preset":"hidream_o1"})).unwrap();
        assert_eq!(hd["6"]["inputs"]["width"], 2048);
        assert_eq!(hd["7"]["inputs"]["noise_scale"], 8);
        for params in [
            json!({"mode":"text2image","preset":"qwen_edit_2511"}),
            json!({"mode":"inpaint","preset":"flux2_klein_4b"}),
            json!({"mode":"img2img","preset":"unknown"}),
            json!({"mode":"text2image","preset":"flux2_klein_4b","checkpoint":"wrong"}),
        ] {
            assert!(graph(&params).is_err());
        }
    }

    #[test]
    fn setup_export_has_matching_model_filenames_and_capability_catalog() {
        let result = setup(
            &mut Session::new(),
            &json!({"mode":"img2img","preset":"qwen_edit_2511","unet":"custom-qwen.safetensors","lora":"custom-lightning.safetensors","steps":12}),
        )
        .unwrap();
        assert_eq!(result["api"]["1"]["inputs"]["unet_name"], "custom-qwen.safetensors");
        let loader = result["workflow"]["nodes"].as_array().unwrap().iter().find(|n| n["id"] == 1).unwrap();
        assert_eq!(loader["widgets_values"][0], "custom-qwen.safetensors");
        let sampler = result["workflow"]["nodes"].as_array().unwrap().iter().find(|n| n["id"] == 35).unwrap();
        assert_eq!(sampler["widgets_values"][2], 12);
        assert_eq!(result["bindings"]["image"]["node"], "10");
        assert_eq!(result["presets"].as_array().unwrap().len(), comfyui::presets::CATALOG.len());
    }
    #[test]
    fn commands_validate_before_network_or_document_changes() {
        let mut s = Session::new();
        for p in [
            json!({}),
            json!({"mode":"bad"}),
            json!({"mode":"text2image","endpoint":false}),
            json!({"mode":"text2image","endpoint":"file:///tmp","prompt":"test"}),
        ] {
            assert!(s.execute("comfyui.run", p).is_err());
        }
        assert!(s.documents().is_empty());
        for p in [
            json!({"mode":"text2image","steps":0}),
            json!({"mode":"img2img","denoise":2}),
            json!({"mode":"text2image","width":65}),
            json!({"mode":"inpaint","workflow":{},"bindings":{}}),
        ] {
            assert!(graph(&p).is_err());
        }
        assert!(s.execute("comfyui.workflow", json!({"mode":"img2img"})).unwrap()["workflow"].is_object());
    }

    #[test]
    fn image_only_custom_workflows_do_not_need_a_prompt_node() {
        let p = json!({"mode":"img2img","workflow":{"1":{"class_type":"LoadImage","inputs":{"image":"source.png"}},"2":{"class_type":"SaveImage","inputs":{"images":["1",0],"filename_prefix":"output"}}},"bindings":{"image":{"node":"1","input":"image"}}});
        assert!(graph(&p).is_ok());
    }
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn inputs_handle_multiple_depths_and_mask_polarity() {
        for (depth, mode) in [8, 16, 32].into_iter().flat_map(|d| ["rgb", "gray", "cmyk", "lab"].into_iter().map(move |m| (d, m))) {
            let mut s = Session::new();
            s.execute("file.new", json!({"width":16,"height":16,"depth":depth,"mode":mode})).unwrap();
            s.execute("select.rect", json!({"x":2,"y":2,"width":4,"height":4})).unwrap();
            let d = &s.active().unwrap().doc;
            let region = region::Region::plan(d, &json!({})).unwrap();
            assert!(!region.input_png(d, false).unwrap().is_empty());
            let mask = photocraft_codecs::decode(&region.input_png(d, true).unwrap()).unwrap();
            assert_eq!(mask.data()[0], 0);
            assert_eq!(mask.data()[((region.pad_top + 3) * region.width + region.pad_left + 3) as usize], 255);
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn generation_and_edits_transfer_assets_and_apply_one_undo_step() {
        use std::io::{BufRead, Read, Write};
        for (preset, mode, selected, resized_output) in comfyui::presets::CATALOG.iter().flat_map(|preset| {
            ["text2image", "img2img", "inpaint"].into_iter().filter(move |mode| preset.supports(mode)).flat_map(move |mode| {
                let mut cases = vec![(preset.id, mode, true, false)];
                if mode == "img2img" {
                    cases.push((preset.id, mode, false, false));
                    cases.push((preset.id, mode, true, true));
                }
                cases
            })
        }) {
            let mut s = Session::new();
            s.execute("file.new", json!({"width":64,"height":48,"depth":16})).unwrap();
            if selected {
                s.execute("select.rect", json!({"x":20,"y":10,"width":4,"height":4})).unwrap();
                std::sync::Arc::make_mut(&mut s.active_mut().unwrap().doc).selection.as_mut().unwrap().write_pixel(20, 10, &[0.5]);
            }
            // The server redraws every pixel, so compositing must enforce the selection.
            let width = if resized_output { 32 } else { 16 };
            let image = photocraft_codecs::Image::from_raw(
                width,
                width,
                photocraft_codecs::ChannelLayout::Rgba,
                photocraft_codecs::SampleType::U8,
                [255u8, 0, 0, 255].repeat(width as usize * width as usize),
            )
            .unwrap();
            let png = photocraft_codecs::encode(&image, photocraft_codecs::Format::Png, &Default::default()).unwrap();
            let original = s.active().unwrap().doc.clone();
            if mode == "text2image" {
                s.close(0);
            }
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();
            let params = json!({"mode":mode,"preset":preset});
            let graph = graph(&params).unwrap();
            let prompt_binding = binding(&params, "prompt", mode).unwrap().unwrap();
            let image_binding = binding(&params, "image", mode).unwrap();
            let info: serde_json::Map<String, Value> =
                graph.as_object().unwrap().values().map(|n| (n["class_type"].as_str().unwrap().into(), json!({"input":{}}))).collect();
            let mut responses = vec![("/object_info", serde_json::to_vec(&Value::Object(info)).unwrap())];
            if mode != "text2image" {
                responses.push(("/upload/image", br#"{"name":"remote-input.png","subfolder":"uploaded"}"#.to_vec()));
            }
            if mode == "inpaint" {
                responses.push(("/upload/image", br#"{"name":"remote-mask.png","subfolder":"uploaded"}"#.to_vec()));
            }
            responses.push(("/prompt", br#"{"prompt_id":"test-id","node_errors":{}}"#.to_vec()));
            responses.push((
                "/history/test-id",
                br#"{"test-id":{"status":{"completed":true},"outputs":{"9":{"images":[{"filename":"result.png","type":"output","subfolder":""}]}}}}"#.to_vec(),
            ));
            responses.push(("/view?", png));
            let edit = mode != "text2image";
            let masked = mode == "inpaint";
            let server = std::thread::spawn(move || {
                for (path, reply) in responses {
                    let (mut stream, _) = listener.accept().unwrap();
                    stream.set_read_timeout(Some(std::time::Duration::from_secs(5))).unwrap();
                    let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
                    let mut first = String::new();
                    reader.read_line(&mut first).unwrap();
                    assert!(first.contains(path), "{first}");
                    let mut length = 0;
                    loop {
                        let mut line = String::new();
                        reader.read_line(&mut line).unwrap();
                        if line == "\r\n" {
                            break;
                        }
                        if let Some(n) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                            length = n.trim().parse::<usize>().unwrap();
                        }
                    }
                    let mut body = vec![0; length];
                    reader.read_exact(&mut body).unwrap();
                    if path == "/prompt" {
                        let request: Value = serde_json::from_slice(&body).unwrap();
                        let prompt = &request["prompt"];
                        assert_eq!(prompt[&prompt_binding.0]["inputs"][&prompt_binding.1], "A watercolor scene");
                        if edit {
                            let (node, input) = image_binding.as_ref().unwrap();
                            assert_eq!(prompt[node]["inputs"][input], "uploaded/remote-input.png");
                        }
                        if masked {
                            assert_eq!(prompt["100"]["inputs"]["image"], "uploaded/remote-mask.png");
                        }
                    }
                    write!(stream, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", reply.len()).unwrap();
                    stream.write_all(&reply).unwrap();
                }
            });
            let result =
                s.execute("comfyui.run", json!({"endpoint":format!("http://{address}"),"mode":mode,"preset":preset,"prompt":"A watercolor scene","context":4}));
            server.join().unwrap();
            let result = result.unwrap();
            assert_eq!(result["promptId"], "test-id");
            let doc = &s.active().unwrap().doc;
            if mode == "text2image" {
                assert_eq!(s.documents().len(), 1);
            } else {
                assert_eq!(doc.layer_count(), original.layer_count() + 1);
                let layer = doc.layer(s.active().unwrap().active_layer.unwrap()).unwrap();
                assert_eq!(layer.surface().unwrap().format().sample, photocraft_color::SampleType::U16);
                assert_eq!(result["selectionMasked"], selected);
                let before = crate::file_cmds::flattened(&original, photocraft_color::PixelFormat::RGBA32F);
                let after = crate::file_cmds::flattened(doc, photocraft_color::PixelFormat::RGBA32F);
                if selected {
                    assert_eq!(layer.mask.as_ref().unwrap().surface, original.selection.as_ref().unwrap().clone());
                    for y in 0..48 {
                        for x in 0..64 {
                            if !(20..24).contains(&x) || !(10..14).contains(&y) {
                                assert_eq!(after.pixel(x, y), before.pixel(x, y), "{preset}: outside selection at {x},{y}");
                            }
                        }
                    }
                    assert_ne!(after.pixel(21, 11), before.pixel(21, 11));
                    assert_ne!(after.pixel(20, 10), before.pixel(20, 10));
                    assert_ne!(after.pixel(20, 10), after.pixel(21, 11), "feathered coverage must blend the result");
                    // The selection hides context pixels without deleting them. Painting
                    // white into the editable layer mask can reveal a generated shadow.
                    let active = s.active().unwrap().active_layer.unwrap();
                    let mut revealed = doc.as_ref().clone();
                    let result_layer = revealed.layer_mut(active).unwrap();
                    assert_eq!(result_layer.surface().unwrap().pixel(19, 11)[3], 1.0, "{preset}: context must retain opaque generated pixels");
                    result_layer.mask.as_mut().unwrap().surface.write_pixel(19, 11, &[1.0]);
                    let exposed = crate::file_cmds::flattened(&revealed, photocraft_color::PixelFormat::RGBA32F);
                    assert_ne!(exposed.pixel(19, 11), before.pixel(19, 11), "{preset}: painting the mask must reveal generated context");
                    assert_eq!(exposed.pixel(18, 11), before.pixel(18, 11), "{preset}: neighboring masked context remains hidden");
                } else {
                    assert!(layer.mask.is_none());
                    assert_ne!(after.pixel(0, 0), before.pixel(0, 0));
                }
                s.execute("edit.undo", json!({})).unwrap();
                assert_eq!(s.active().unwrap().doc.layer_count(), original.layer_count());
            }
        }
    }
}
