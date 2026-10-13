//! ComfyUI's self-hosted HTTP protocol. Never installs nodes or models.
use crate::{Error, Result};
use serde_json::{Value, json};

pub mod presets;

pub const MAX_GRAPH_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_IMAGE_BYTES: usize = 64 * 1024 * 1024;

fn bounded_value(value: &Value, depth: usize, budget: &mut usize) -> Result<()> {
    if depth > 64 {
        return Err(Error::Limit("ComfyUI workflow nesting (64)".into()));
    }
    *budget = budget.saturating_add(match value {
        Value::String(s) => s.len(),
        _ => 16,
    });
    if *budget > MAX_GRAPH_BYTES {
        return Err(Error::Limit("ComfyUI workflow size".into()));
    }
    match value {
        Value::Object(fields) => {
            for (key, child) in fields {
                *budget = budget.saturating_add(key.len());
                bounded_value(child, depth + 1, budget)?;
            }
        }
        Value::Array(items) => {
            for child in items {
                bounded_value(child, depth + 1, budget)?;
            }
        }
        _ => {}
    }
    Ok(())
}

/// Bundled editable UI workflow and executable API graph.
pub fn workflow(mode: &str) -> Result<(Value, Value)> {
    let (ui, api) = match mode {
        "text2image" => (include_str!("../workflows/comfyui/text2image.json"), include_str!("../workflows/comfyui/text2image.api.json")),
        "img2img" => (include_str!("../workflows/comfyui/img2img.json"), include_str!("../workflows/comfyui/img2img.api.json")),
        "inpaint" => (include_str!("../workflows/comfyui/inpaint.json"), include_str!("../workflows/comfyui/inpaint.api.json")),
        _ => return Err(Error::Params("mode must be text2image, img2img or inpaint".into())),
    };
    Ok((serde_json::from_str(ui).map_err(failed)?, serde_json::from_str(api).map_err(failed)?))
}

fn failed(e: impl std::fmt::Display) -> Error {
    Error::Failed(format!("ComfyUI: {e}"))
}

/// Reject UI-format exports and malformed/oversized API graphs before sending assets.
pub fn validate(graph: &Value) -> Result<()> {
    bounded_value(graph, 0, &mut 0)?;
    let nodes = graph
        .as_object()
        .filter(|m| !m.is_empty() && m.len() <= 1024)
        .ok_or_else(|| Error::Params("workflow must be an API-format graph with 1–1024 nodes (export API format in ComfyUI)".into()))?;
    if serde_json::to_vec(graph).map_err(failed)?.len() > MAX_GRAPH_BYTES {
        return Err(Error::Limit("ComfyUI workflow size".into()));
    }
    for (id, node) in nodes {
        if id.len() > 128 || node.get("class_type").and_then(Value::as_str).is_none_or(str::is_empty) {
            return Err(Error::Params(format!("invalid workflow node {id}")));
        }
        let inputs = node.get("inputs").and_then(Value::as_object).ok_or_else(|| Error::Params(format!("node {id} has no inputs object")))?;
        for value in inputs.values() {
            if let Some(link) = value.as_array() {
                let source = link.first().and_then(Value::as_str);
                if link.len() != 2 || source.is_none_or(|s| !nodes.contains_key(s)) || link.get(1).and_then(Value::as_u64).is_none() {
                    return Err(Error::Params(format!("node {id} has an invalid link")));
                }
            }
        }
    }
    // Kahn's algorithm rejects cycles without recursion, including disconnected cycles.
    let mut indegree: std::collections::HashMap<&str, usize> = nodes.keys().map(|id| (id.as_str(), 0)).collect();
    let mut successors: std::collections::HashMap<&str, Vec<&str>> = std::collections::HashMap::new();
    for (id, node) in nodes {
        if let Some(inputs) = node.get("inputs").and_then(Value::as_object) {
            for value in inputs.values() {
                if let Some(source) = value.as_array().and_then(|v| v.first()).and_then(Value::as_str) {
                    if let Some(count) = indegree.get_mut(id.as_str()) {
                        *count = count.saturating_add(1);
                    }
                    successors.entry(source).or_default().push(id);
                }
            }
        }
    }
    let mut ready: Vec<&str> = indegree.iter().filter(|(_, count)| **count == 0).map(|(id, _)| *id).collect();
    let mut visited = 0usize;
    while let Some(id) = ready.pop() {
        visited = visited.saturating_add(1);
        for next in successors.get(id).into_iter().flatten() {
            if let Some(count) = indegree.get_mut(next) {
                *count = count.saturating_sub(1);
                if *count == 0 {
                    ready.push(next);
                }
            }
        }
    }
    if visited != nodes.len() {
        return Err(Error::Params("ComfyUI workflow contains a cycle".into()));
    }
    Ok(())
}

/// Explicit bindings support model-specific API workflows without guessing widget ordering.
pub fn bind(graph: &mut Value, node: &str, input: &str, value: Value) -> Result<()> {
    let slot = graph
        .get_mut(node)
        .and_then(|n| n.get_mut("inputs"))
        .and_then(|i| i.get_mut(input))
        .ok_or_else(|| Error::Params(format!("workflow has no input {node}.{input}")))?;
    *slot = value;
    Ok(())
}

/// Missing classes and unavailable enum values (including checkpoint/model filenames).
/// Availability is discovery, not a guarantee that a model can load or fit in memory.
pub fn availability(graph: &Value, info: &Value) -> Result<Value> {
    validate(graph)?;
    let nodes = graph.as_object().ok_or_else(|| Error::Params("workflow is not an object".into()))?;
    let mut missing_nodes = Vec::new();
    let mut missing_inputs = Vec::new();
    for (id, node) in nodes {
        let class = node.get("class_type").and_then(Value::as_str).unwrap_or_default();
        let Some(spec) = info.get(class) else {
            missing_nodes.push(json!({"node":id,"class":class}));
            continue;
        };
        for section in ["required", "optional"] {
            if let Some(inputs) = spec.get("input").and_then(|v| v.get(section)).and_then(Value::as_object) {
                for (name, schema) in inputs {
                    // Uploaded image filenames are discovered only after uploading.
                    if matches!(class, "LoadImage" | "LoadImageMask") && name == "image" {
                        continue;
                    }
                    let value = node.get("inputs").and_then(|v| v.get(name));
                    let choices = schema
                        .get(0)
                        .and_then(Value::as_array)
                        .or_else(|| (schema.get(0).and_then(Value::as_str) == Some("COMBO")).then(|| schema.get(1)?.get("options")?.as_array()).flatten());
                    if let (Some(value), Some(choices)) = (value, choices)
                        && !value.is_array()
                        && !choices.contains(value)
                    {
                        // A loader can enumerate thousands of models. Do not duplicate that
                        // remote list once per workflow node in the discovery report.
                        missing_inputs.push(json!({"node":id,"class":class,"input":name,"value":value,"availableCount":choices.len()}));
                    }
                }
            }
        }
    }
    Ok(json!({"ready":missing_nodes.is_empty() && missing_inputs.is_empty(),"missingNodes":missing_nodes,"missingInputs":missing_inputs,
        "guidance":"Open the setup workflow in ComfyUI, install its missing nodes/models there, then check again."}))
}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use super::*;
    use reqwest::{
        Url,
        blocking::{Client as Http, Response, multipart},
    };
    use std::{io::Read, time::Duration};

    pub struct Client {
        http: Http,
        base: Url,
    }

    impl Client {
        pub fn new(endpoint: &str) -> Result<Self> {
            let mut base = Url::parse(endpoint).map_err(failed)?;
            if !matches!(base.scheme(), "http" | "https")
                || base.host_str().is_none()
                || !base.username().is_empty()
                || base.password().is_some()
                || base.query().is_some()
                || base.fragment().is_some()
            {
                return Err(Error::Params("ComfyUI endpoint must be an http(s) URL without credentials, query or fragment".into()));
            }
            if !base.path().ends_with('/') {
                base.set_path(&format!("{}/", base.path()));
            }
            let http = Http::builder()
                .timeout(Duration::from_secs(15))
                .connect_timeout(Duration::from_secs(5))
                .redirect(reqwest::redirect::Policy::none())
                .user_agent("Photocraft-dev")
                .build()
                .map_err(failed)?;
            Ok(Self { http, base })
        }

        fn url(&self, path: &str) -> Result<Url> {
            self.base.join(path).map_err(failed)
        }

        fn bytes(response: Response, limit: usize) -> Result<Vec<u8>> {
            let status = response.status();
            let mut out = Vec::new();
            response.take(limit as u64 + 1).read_to_end(&mut out).map_err(failed)?;
            if out.len() > limit {
                return Err(Error::Limit("ComfyUI response size".into()));
            }
            if !status.is_success() {
                // Include ComfyUI's validation error and per-node diagnostics.
                let message: String = String::from_utf8_lossy(&out).chars().take(4096).collect();
                return Err(failed(format!("HTTP {status}: {message}")));
            }
            Ok(out)
        }

        fn json(response: Response) -> Result<Value> {
            serde_json::from_slice(&Self::bytes(response, MAX_GRAPH_BYTES)?).map_err(failed)
        }

        pub fn object_info(&self) -> Result<Value> {
            Self::json(self.http.get(self.url("object_info")?).send().map_err(failed)?)
        }

        /// Uploads via HTTP even when the server runs on this machine.
        pub fn upload(&self, name: &str, bytes: Vec<u8>) -> Result<String> {
            if bytes.len() > MAX_IMAGE_BYTES {
                return Err(Error::Limit("ComfyUI input image size".into()));
            }
            let part = multipart::Part::bytes(bytes).file_name(name.to_string()).mime_str("image/png").map_err(failed)?;
            let form = multipart::Form::new().part("image", part).text("type", "input").text("overwrite", "false");
            let reply = Self::json(self.http.post(self.url("upload/image")?).multipart(form).send().map_err(failed)?)?;
            let name = reply.get("name").and_then(Value::as_str).filter(|s| !s.is_empty()).ok_or_else(|| failed("upload returned no name"))?;
            let subfolder = reply.get("subfolder").and_then(Value::as_str).unwrap_or_default();
            Ok(if subfolder.is_empty() { name.to_string() } else { format!("{subfolder}/{name}") })
        }

        pub fn submit(&self, graph: &Value) -> Result<String> {
            validate(graph)?;
            let reply = Self::json(self.http.post(self.url("prompt")?).json(&json!({"prompt":graph})).send().map_err(failed)?)?;
            if reply.get("node_errors").and_then(Value::as_object).is_some_and(|m| !m.is_empty()) {
                return Err(failed(reply));
            }
            reply
                .get("prompt_id")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty() && s.len() <= 128)
                .map(str::to_string)
                .ok_or_else(|| failed("queue returned no prompt_id"))
        }

        pub fn history(&self, id: &str) -> Result<Value> {
            let mut url = self.url("history/")?;
            url.path_segments_mut().map_err(|_| failed("invalid history URL"))?.pop_if_empty().push(id);
            Self::json(self.http.get(url).send().map_err(failed)?)
        }

        /// Removes only this pending prompt; never interrupts another client's running work.
        pub fn cancel_pending(&self, id: &str) -> Result<()> {
            Self::json(self.http.post(self.url("queue")?).json(&json!({"delete":[id]})).send().map_err(failed)?)?;
            Ok(())
        }

        pub fn download(&self, image: &Value) -> Result<Vec<u8>> {
            let name = image.get("filename").and_then(Value::as_str).filter(|s| !s.is_empty()).ok_or_else(|| failed("output has no filename"))?;
            let mut url = self.url("view")?;
            url.query_pairs_mut()
                .append_pair("filename", name)
                .append_pair("subfolder", image.get("subfolder").and_then(Value::as_str).unwrap_or_default())
                .append_pair("type", image.get("type").and_then(Value::as_str).unwrap_or("output"));
            Self::bytes(self.http.get(url).send().map_err(failed)?, MAX_IMAGE_BYTES)
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub use native::Client;

/// None while queued/running, output descriptors after success, error after failure.
pub fn outputs(history: &Value, id: &str) -> Result<Option<Vec<Value>>> {
    let Some(entry) = history.get(id) else { return Ok(None) };
    let status = entry.get("status").ok_or_else(|| failed("history has no status"))?;
    if status.get("status_str").and_then(Value::as_str) == Some("error") {
        return Err(failed(status));
    }
    if status.get("completed").and_then(Value::as_bool) != Some(true) {
        return Ok(None);
    }
    let mut images = Vec::new();
    if let Some(nodes) = entry.get("outputs").and_then(Value::as_object) {
        for node in nodes.values() {
            if let Some(list) = node.get("images").and_then(Value::as_array) {
                images.extend(list.iter().cloned());
                if images.len() > 16 {
                    return Err(Error::Limit("ComfyUI output count (16)".into()));
                }
            }
        }
    }
    if images.is_empty() {
        return Err(failed("workflow completed without images; add SaveImage or PreviewImage"));
    }
    Ok(Some(images))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bundled_graphs_and_bindings_are_valid() {
        for mode in ["text2image", "img2img", "inpaint"] {
            let (ui, mut graph) = workflow(mode).unwrap();
            assert!(ui.get("nodes").unwrap().is_array());
            validate(&graph).unwrap();
            bind(&mut graph, "6", "text", json!("test")).unwrap();
            assert!(bind(&mut graph, "missing", "text", json!("test")).is_err());
        }
        assert!(workflow("unknown").is_err());
        for bad in [json!({}), json!([]), json!({"nodes":[]}), json!({"1":{"class_type":"Test","inputs":{"image":["missing",0]}}})] {
            assert!(validate(&bad).is_err());
        }
    }
    #[test]
    fn reports_missing_nodes_and_models() {
        let graph =
            json!({"1":{"class_type":"CheckpointLoaderSimple","inputs":{"ckpt_name":"missing.safetensors"}},"2":{"class_type":"CustomEdit","inputs":{}}});
        let info = json!({"CheckpointLoaderSimple":{"input":{"required":{"ckpt_name":[["installed.safetensors"]]}}}});
        let result = availability(&graph, &info).unwrap();
        assert_eq!(result["ready"], false);
        assert_eq!(result["missingNodes"][0]["class"], "CustomEdit");
        assert_eq!(result["missingInputs"][0]["input"], "ckpt_name");
    }

    #[test]
    fn availability_does_not_multiply_remote_model_lists() {
        let graph = json!({"1":{"class_type":"Loader","inputs":{"model":"missing"}},"2":{"class_type":"Loader","inputs":{"model":"missing"}}});
        let info = json!({"Loader":{"input":{"required":{"model":[["x".repeat(65536)]]}}}});
        let report = availability(&graph, &info).unwrap();
        assert_eq!(report["missingInputs"][0]["availableCount"], 1);
        assert!(serde_json::to_vec(&report).unwrap().len() < 1024);
    }
    #[test]
    fn history_handles_pending_failed_empty_and_successful_jobs() {
        assert!(outputs(&json!({}), "id").unwrap().is_none());
        assert!(outputs(&json!({"id":{"status":{"status_str":"error","messages":[["execution_error",{}]]}}}), "id").is_err());
        assert!(outputs(&json!({"id":{"status":{"completed":true},"outputs":{}}}), "id").is_err());
        assert_eq!(
            outputs(&json!({"id":{"status":{"completed":true},"outputs":{"9":{"images":[{"filename":"result.png"}]}}}}), "id").unwrap().unwrap().len(),
            1
        );
    }
    #[test]
    fn cyclic_graphs_are_rejected_without_recursion() {
        assert!(validate(&json!({"1":{"class_type":"Test","inputs":{"x":["2",0]}},"2":{"class_type":"Test","inputs":{"x":["1",0]}}})).is_err());
    }

    #[test]
    fn deeply_nested_metadata_is_refused_before_serialization_or_cloning() {
        let mut nested = json!(null);
        for _ in 0..70 {
            nested = json!({"child":nested});
        }
        assert!(validate(&json!({"1":{"class_type":"Test","inputs":{},"_meta":nested}})).is_err());
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn http_uses_upload_queue_history_and_view_with_remote_subpaths() {
        use std::io::{BufRead, Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            for (path, body, reply) in [
                ("GET /comfy/object_info", "", "{\"Node\":{}}"),
                ("POST /comfy/upload/image", "photocraft-input.png", "{\"name\":\"renamed image.png\",\"subfolder\":\"inputs\",\"type\":\"input\"}"),
                ("POST /comfy/prompt", "class_type", "{\"prompt_id\":\"job-1\",\"node_errors\":{}}"),
                ("GET /comfy/history/job-1", "", "{}"),
                ("GET /comfy/view?filename=a+b%26c.png&subfolder=folder&type=output", "", "PNG"),
                ("POST /comfy/queue", "job-1", "{}"),
            ] {
                let (mut stream, _) = listener.accept().unwrap();
                stream.set_read_timeout(Some(std::time::Duration::from_secs(5))).unwrap();
                let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
                let mut first = String::new();
                reader.read_line(&mut first).unwrap();
                assert!(first.starts_with(path), "{first}");
                let mut length = 0;
                loop {
                    let mut header = String::new();
                    reader.read_line(&mut header).unwrap();
                    if header == "\r\n" {
                        break;
                    }
                    if let Some(n) = header.to_ascii_lowercase().strip_prefix("content-length:") {
                        length = n.trim().parse::<usize>().unwrap();
                    }
                }
                let mut data = vec![0; length];
                reader.read_exact(&mut data).unwrap();
                assert!(String::from_utf8_lossy(&data).contains(body));
                write!(stream, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}", reply.len()).unwrap();
            }
        });
        let client = Client::new(&format!("http://{address}/comfy")).unwrap();
        assert!(client.object_info().unwrap().get("Node").is_some());
        assert_eq!(client.upload("photocraft-input.png", vec![1, 2, 3]).unwrap(), "inputs/renamed image.png");
        assert_eq!(client.submit(&json!({"1":{"class_type":"Node","inputs":{}}})).unwrap(), "job-1");
        assert_eq!(client.history("job-1").unwrap(), json!({}));
        assert_eq!(client.download(&json!({"filename":"a b&c.png","subfolder":"folder","type":"output"})).unwrap(), b"PNG");
        client.cancel_pending("job-1").unwrap();
        server.join().unwrap();
        for invalid in ["file:///tmp", "http://user:secret@host", "http://host/?query=yes"] {
            assert!(Client::new(invalid).is_err());
        }
    }
}
