//! Model capabilities and single-reference graphs, adapted from official ComfyUI templates.
use crate::{Error, Result};
use serde_json::Value;

pub const SOURCE: &str = "https://github.com/Comfy-Org/workflow_templates/tree/8be1f8c4b5af2d550d70922a23b79cee599e1f3e";

pub const HIDREAM_SIZES: &[(u32, u32)] =
    &[(2048, 2048), (2304, 1728), (2304, 1792), (2496, 1664), (2560, 1440), (3104, 1312), (1728, 2304), (1792, 2304), (1664, 2496), (1440, 2560), (1312, 3104)];

pub struct Preset {
    pub id: &'static str,
    pub label: &'static str,
    pub generate: bool,
    pub steps: u32,
    pub cfg: f64,
    pub size: u32,
    data: &'static str,
}

pub const CATALOG: &[Preset] = &[
    Preset {
        id: "sdxl_union_inpaint",
        label: "SDXL Union ProMax Inpaint (Xinsir)",
        generate: false,
        steps: 25,
        cfg: 4.5,
        size: 1024,
        data: include_str!("../../workflows/comfyui/sdxl_union_inpaint.json"),
    },
    Preset {
        id: "sdxl_controlnet_inpaint",
        label: "SDXL Inpaint ControlNet (Dreamer)",
        generate: false,
        steps: 15,
        cfg: 8.0,
        size: 1024,
        data: include_str!("../../workflows/comfyui/sdxl_controlnet_inpaint.json"),
    },
    Preset {
        id: "sdxl_inpaint",
        label: "SDXL 1.0 Inpaint",
        generate: false,
        steps: 30,
        cfg: 7.0,
        size: 1024,
        data: include_str!("../../workflows/comfyui/sdxl_inpaint.json"),
    },
    Preset { id: "sd", label: "Stable Diffusion starter", generate: true, steps: 20, cfg: 8.0, size: 512, data: "" },
    Preset {
        id: "flux2_klein_4b",
        label: "FLUX.2 Klein 4B",
        generate: true,
        steps: 4,
        cfg: 1.0,
        size: 1024,
        data: include_str!("../../workflows/comfyui/flux2_klein_4b.json"),
    },
    Preset {
        id: "flux2_klein_9b",
        label: "FLUX.2 Klein 9B",
        generate: true,
        steps: 4,
        cfg: 1.0,
        size: 1024,
        data: include_str!("../../workflows/comfyui/flux2_klein_9b.json"),
    },
    Preset {
        id: "qwen_edit_2511",
        label: "Qwen Image Edit 2511 (INT8 + Lightning)",
        generate: false,
        steps: 8,
        cfg: 1.0,
        size: 1024,
        data: include_str!("../../workflows/comfyui/qwen_edit_2511.json"),
    },
    Preset {
        id: "qwen_edit_2511_gguf",
        label: "Qwen Image Edit 2511 (GGUF + Lightning)",
        generate: false,
        steps: 8,
        cfg: 1.0,
        size: 1024,
        data: include_str!("../../workflows/comfyui/qwen_edit_2511_gguf.json"),
    },
    Preset {
        id: "qwen_edit_2511_fp8_4step",
        label: "Qwen Image Edit 2511 (FP8 + 4-step Lightning)",
        generate: false,
        steps: 4,
        cfg: 1.0,
        size: 1024,
        data: include_str!("../../workflows/comfyui/qwen_edit_2511_fp8_4step.json"),
    },
    Preset {
        id: "mage_flow_edit",
        label: "Mage Flow Edit",
        generate: false,
        steps: 30,
        cfg: 5.0,
        size: 1024,
        data: include_str!("../../workflows/comfyui/mage_flow_edit.json"),
    },
    Preset {
        id: "mage_flow_edit_turbo",
        label: "Mage Flow Edit Turbo",
        generate: false,
        steps: 4,
        cfg: 1.0,
        size: 1024,
        data: include_str!("../../workflows/comfyui/mage_flow_edit_turbo.json"),
    },
    Preset {
        id: "hidream_o1",
        label: "HiDream O1",
        generate: true,
        steps: 40,
        cfg: 5.0,
        size: 2048,
        data: include_str!("../../workflows/comfyui/hidream_o1.json"),
    },
];

impl Preset {
    pub fn supports(&self, mode: &str) -> bool {
        match mode {
            "text2image" => self.generate,
            "img2img" => !matches!(self.id, "sdxl_inpaint" | "sdxl_controlnet_inpaint" | "sdxl_union_inpaint"),
            "inpaint" => matches!(self.id, "sd" | "sdxl_inpaint" | "sdxl_controlnet_inpaint" | "sdxl_union_inpaint"),
            _ => false,
        }
    }

    /// Model-specific widget bindings and editable frontend workflow accompany the API graph.
    pub fn workflow(&self, mode: &str) -> Result<Value> {
        if !self.supports(mode) {
            return Err(Error::Params(format!("{} does not support {mode}; choose a supported mode or a custom workflow", self.label)));
        }
        if self.id == "sd" {
            let (ui, api) = super::workflow(mode)?;
            let mut bindings = serde_json::Map::new();
            for (key, node, input) in [
                ("prompt", "6", "text"),
                ("negative", "7", "text"),
                ("checkpoint", if mode == "inpaint" { "29" } else { "4" }, "ckpt_name"),
                ("seed", "3", "seed"),
                ("steps", "3", "steps"),
                ("cfg", "3", "cfg"),
                ("denoise", "3", "denoise"),
            ] {
                let widget = match key {
                    "steps" => 2,
                    "cfg" => 3,
                    "denoise" => 6,
                    _ => 0,
                };
                bindings.insert(key.into(), serde_json::json!({"node":node,"input":input,"widget":widget}));
            }
            if mode == "text2image" {
                for key in ["width", "height"] {
                    bindings.insert(key.into(), serde_json::json!({"node":"5","input":key,"widget":if key == "width" {0} else {1}}));
                }
            } else {
                bindings.insert("image".into(), serde_json::json!({"node":if mode == "inpaint" {"20"} else {"100"},"input":"image"}));
                if mode == "inpaint" {
                    bindings.insert("mask".into(), serde_json::json!({"node":"100","input":"image"}));
                }
            }
            return Ok(serde_json::json!({"ui":ui,"api":api,"bindings":bindings}));
        }
        let bundle: Value = serde_json::from_str(self.data).map_err(super::failed)?;
        bundle.get(mode).cloned().ok_or_else(|| Error::Params(format!("Missing workflow for {} / {mode}", self.id)))
    }
}

pub fn get(id: &str) -> Result<&'static Preset> {
    CATALOG.iter().find(|preset| preset.id == id).ok_or_else(|| Error::Params(format!("Unknown ComfyUI preset: {id}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn union_inpaint_checks_promax_weights_and_repaint_combo() {
        let preset = get("sdxl_union_inpaint").unwrap();
        assert!(!preset.supports("img2img"));
        assert!(!preset.supports("text2image"));
        let bundle = preset.workflow("inpaint").unwrap();
        let api = &bundle["api"];
        let mut info: serde_json::Map<String, Value> =
            api.as_object().unwrap().values().map(|n| (n["class_type"].as_str().unwrap().into(), serde_json::json!({"input":{}}))).collect();
        info.insert("ControlNetLoader".into(), serde_json::json!({"input":{"required":{"control_net_name":[["diffusion_pytorch_model_promax.safetensors"]]}}}));
        info.insert("SetUnionControlNetType".into(), serde_json::json!({"input":{"required":{"type":["COMBO",{"options":["auto","repaint"]}]}}}));
        let mut info = Value::Object(info);
        assert_eq!(super::super::availability(api, &info).unwrap()["ready"], true);
        info["ControlNetLoader"]["input"]["required"]["control_net_name"] = serde_json::json!([["diffusion_pytorch_model.safetensors"]]);
        assert_eq!(super::super::availability(api, &info).unwrap()["missingInputs"][0]["input"], "control_net_name");
        info["SetUnionControlNetType"]["input"]["required"]["type"][1]["options"] = serde_json::json!(["auto", "tile"]);
        assert_eq!(super::super::availability(api, &info).unwrap()["missingInputs"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn controlnet_inpaint_checks_the_base_and_control_model_separately() {
        let preset = get("sdxl_controlnet_inpaint").unwrap();
        assert!(!preset.supports("img2img"));
        assert!(!preset.supports("text2image"));
        let bundle = preset.workflow("inpaint").unwrap();
        let api = &bundle["api"];
        let mut info = serde_json::Map::new();
        for node in api.as_object().unwrap().values() {
            info.insert(node["class_type"].as_str().unwrap().into(), serde_json::json!({"input":{"required":{}}}));
        }
        info.insert("CheckpointLoaderSimple".into(), serde_json::json!({"input":{"required":{"ckpt_name":[["sd_xl_base_1.0.safetensors"]]}}}));
        info.insert("ControlNetLoader".into(), serde_json::json!({"input":{"required":{"control_net_name":[["diffusion_pytorch_model.fp16.safetensors"]]}}}));
        let mut info = Value::Object(info);
        assert_eq!(super::super::availability(api, &info).unwrap()["ready"], true);
        info["ControlNetLoader"]["input"]["required"]["control_net_name"] = serde_json::json!([[]]);
        let report = super::super::availability(api, &info).unwrap();
        assert_eq!(report["missingInputs"][0]["input"], "control_net_name");
        info.as_object_mut().unwrap().remove("ControlNetApply");
        assert_eq!(super::super::availability(api, &info).unwrap()["missingNodes"][0]["class"], "ControlNetApply");
    }

    #[test]
    fn all_presets_have_valid_links_bindings_and_frontend_exports() {
        for preset in CATALOG {
            for mode in ["text2image", "img2img", "inpaint"] {
                if !preset.supports(mode) {
                    assert!(preset.workflow(mode).is_err());
                    continue;
                }
                let bundle = preset.workflow(mode).unwrap();
                super::super::validate(&bundle["api"]).unwrap();
                for (key, binding) in bundle["bindings"].as_object().unwrap() {
                    let node = binding["node"].as_str().unwrap();
                    let input = binding["input"].as_str().unwrap();
                    assert!(bundle["api"][node]["inputs"].get(input).is_some(), "{} {mode} {key}", preset.id);
                    if preset.id != "sd" {
                        let frontend = bundle["ui"]["nodes"].as_array().unwrap().iter().find(|n| n["id"].as_u64().unwrap().to_string() == node).unwrap();
                        let widget = binding["widget"].as_u64().unwrap() as usize;
                        assert_eq!(frontend["widgets_values"][widget], bundle["api"][node]["inputs"][input], "{} {key}", preset.id);
                    }
                }
                let nodes = bundle["ui"]["nodes"].as_array().unwrap();
                assert!(!nodes.is_empty());
                for link in bundle["ui"]["links"].as_array().unwrap() {
                    let from = nodes.iter().find(|n| n["id"] == link[1]).unwrap();
                    let to = nodes.iter().find(|n| n["id"] == link[3]).unwrap();
                    assert!(from["outputs"][link[2].as_u64().unwrap() as usize]["links"].as_array().unwrap().contains(&link[0]));
                    assert_eq!(to["inputs"][link[4].as_u64().unwrap() as usize]["link"], link[0]);
                }
            }
        }
        assert!(get("unknown").is_err());
        assert!(!get("qwen_edit_2511").unwrap().supports("text2image"));
    }

    #[test]
    fn availability_checks_split_models_loras_and_gguf_nodes() {
        let bundle = get("qwen_edit_2511_gguf").unwrap().workflow("img2img").unwrap();
        let graph = &bundle["api"];
        let mut info = serde_json::Map::new();
        for node in graph.as_object().unwrap().values() {
            let mut inputs = serde_json::Map::new();
            for (name, value) in node["inputs"].as_object().unwrap() {
                if matches!(name.as_str(), "unet_name" | "clip_name" | "vae_name" | "lora_name") {
                    inputs.insert(name.clone(), serde_json::json!([[value], {}]));
                }
            }
            info.insert(node["class_type"].as_str().unwrap().into(), serde_json::json!({"input":{"required":inputs}}));
        }
        let mut info = Value::Object(info);
        assert_eq!(super::super::availability(graph, &info).unwrap()["ready"], true);
        info["LoraLoaderModelOnly"]["input"]["required"]["lora_name"] = serde_json::json!([["different.safetensors"], {}]);
        info.as_object_mut().unwrap().remove("UnetLoaderGGUF");
        let report = super::super::availability(graph, &info).unwrap();
        assert_eq!(report["ready"], false);
        assert_eq!(report["missingNodes"][0]["class"], "UnetLoaderGGUF");
        assert_eq!(report["missingInputs"][0]["input"], "lora_name");
    }

    #[test]
    fn qwen_outputs_use_original_image_size_after_preferred_resolution_inference() {
        for id in ["qwen_edit_2511", "qwen_edit_2511_gguf", "qwen_edit_2511_fp8_4step"] {
            let bundle = get(id).unwrap().workflow("img2img").unwrap();
            let api = &bundle["api"];
            assert_eq!(api["21"]["inputs"]["pixels"], serde_json::json!(["20", 0]));
            assert_eq!(api["40"]["inputs"]["image"], serde_json::json!([bundle["bindings"]["image"]["node"], 0]));
            assert_eq!(api["41"]["inputs"]["width"], serde_json::json!(["40", 0]));
            assert_eq!(api["41"]["inputs"]["height"], serde_json::json!(["40", 1]));
            assert_eq!(api["37"]["inputs"]["images"], serde_json::json!(["41", 0]));
        }
    }
}
