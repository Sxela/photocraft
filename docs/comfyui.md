# ComfyUI integration

PhotoCraft's native ComfyUI integration is a host-side workflow plugin, separate from the
network-free WebAssembly filter ABI. Click **ComfyUI** in the top bar (or open
**Window → ComfyUI Generate / Edit**), then choose **Setup** and enter a
local or remote self-hosted ComfyUI URL, such as `http://127.0.0.1:8188` or
`https://images.example.com/comfy/`. HTTPS uses normal certificate validation. Assets always
travel over HTTP; the two applications do not need a shared filesystem.

## First run

1. Choose a **Model**, then open **Setup** and choose a supported mode.
2. Use **Save Starter Workflow…**, open that JSON in ComfyUI, and install the dependencies
   there. PhotoCraft never installs ComfyUI, nodes, checkpoints or other models.
3. Enter your installed model filenames, or leave them blank to use the preset defaults.
   Split models have separate diffusion-model, text-encoder, VAE and optional LoRA fields.
   Click **Check Availability**. Missing nodes and unavailable model
   choices appear with instructions to resolve them in ComfyUI.
4. Switch to **Generate / Edit**, enter a prompt and run. Connection, checkpoint and custom
   workflow configuration stay in Setup; return there when you want to adjust them.
   Progress, elapsed time and Cancel appear inside the ComfyUI window while a job runs.
   Generation does not open a blocking progress popup; you can switch documents or adjust
   setup while it runs. The input document stays locked against edits until the job ends.
   Closing the ComfyUI window keeps the job running; status-bar progress remains available.
   When availability reports missing dependencies, **Save Starter Workflow…** also appears
   directly below the status in Generate / Edit. Save the JSON, then drag it onto ComfyUI's
   canvas or open it there. The export is an editable frontend workflow for the selected model
   and mode, including your model filename overrides, rather than the API prompt graph.
   The selected preset and each preset's filename overrides are saved with PhotoCraft preferences
   and restored after restarting; switching models preserves those overrides. The panel opens
   on Generate / Edit with a fresh prompt.
   Edits upload the selected area's bounding rectangle plus surrounding pixels (128 per side
   by default, clipped to the canvas). With no selection, Edit image uses the whole canvas.
   Inputs larger than the processing limit are reduced proportionally; smaller inputs are
   centered and padded with repeated edge pixels to at least 1024x1024. HiDream uses 2048x2048.
   Processing dimensions are aligned to 16 pixels and never exceed 2048x2048.
   Set **Surrounding pixels** and **Maximum processing side** in Setup. Generate / Edit shows
   the context and processing dimensions; width/height controls apply only to text generation.
   Inpainting uploads an identically transformed grayscale mask: white edits, black preserves,
   and all padding is black. SD/SDXL condition on that mask; instruction models receive image
   context and apply the original selection afterwards.
   Results are unpadded, resized back to the context rectangle and placed at its original
   document coordinates as new raster layers in one undo step. Both edit modes retain the
   captured selection as a layer mask, including feathered edges, so outside pixels are preserved.
   The result layer keeps all generated pixels in the surrounding context rectangle underneath
   that editable mask. Deselect, select the result's mask thumbnail, and paint white or feather
   the mask to reveal nearby generated details such as shadows. Increasing **Surrounding pixels**
   before running gives the model more real canvas context and retains a larger editable result.
   Padding to the 1024 minimum repeats edge pixels; it is removed on import and does not add
   real surrounding canvas content. Mask edits can reveal only details present in the returned
   image; an inpaint workflow may preserve pixels outside its input mask.
   An unexpected output aspect ratio is rejected rather than stretched. Custom workflows must
   produce the uploaded input's aspect ratio and bind output width/height where applicable.
   Restoration is bounded to 512 MiB at the document's native depth, 32768 pixels per side,
   and 64 megapixels across results; encoded inference outputs remain limited to 16 megapixels.
   With no document open, text generation opens new documents.

The bundled starters derive from Comfy Org's official workflow templates at commit
`8be1f8c4b5af2d550d70922a23b79cee599e1f3e`: `archived/default.json` (generation and the
img2img adaptation) and `archived/inpaint_example.json` (inpaint adaptation). Their permissive
license and attribution ship with the JSONs. Img2img replaces EmptyLatentImage with an uploaded
image and VAEEncode. Inpaint removes automatic resizing and replaces the image-alpha mask
with an independent LoadImageMask using its red channel. No model weights are bundled.

## Bundled model presets

These use the model filenames and sampling settings wired into VibeWarp's ComfyUI backends,
with single-image graphs independently adapted from Comfy Org's official templates at the
same pinned commit. They require no shared filesystem or VibeWarp installation at runtime.

| Preset ID | Model | Generate | Edit image | Model inpaint | Defaults |
|---|---|---|---|---|---|
| `sdxl_union_inpaint` | SDXL Union ProMax Inpaint (Xinsir) | No | No | Yes | 25 steps, CFG 4.5, 1024 minimum |
| `sdxl_controlnet_inpaint` | SDXL Inpaint ControlNet (Dreamer) | No | No | Yes | 15 steps, CFG 8, 1024 minimum |
| `sdxl_inpaint` | SDXL 1.0 Inpaint | No | No | Yes | 30 steps, CFG 7, 1024 minimum |
| `sd` | Stable Diffusion starter | Yes | Yes | Yes | 20 steps, CFG 8, 512 square |
| `flux2_klein_4b` | FLUX.2 Klein 4B distilled | Yes | Yes | — | 4 steps, CFG 1, 1024 square |
| `flux2_klein_9b` | FLUX.2 Klein 9B distilled | Yes | Yes | — | 4 steps, CFG 1, 1024 square |
| `qwen_edit_2511` | Qwen Image Edit 2511 INT8 + Lightning | — | Yes | — | 8 steps, CFG 1, source-sized output |
| `qwen_edit_2511_gguf` | Qwen Image Edit 2511 Q5_K_M + Lightning | — | Yes | — | 8 steps, CFG 1, source-sized output |
| `qwen_edit_2511_fp8_4step` | Qwen Image Edit 2511 FP8 + Lightning | — | Yes | — | 4 steps, CFG 1, source-sized output |
| `mage_flow_edit` | Mage Flow Edit INT8 | — | Yes | — | 30 steps, CFG 5, 1024 square |
| `mage_flow_edit_turbo` | Mage Flow Edit Turbo INT8 | — | Yes | — | 4 steps, CFG 1, 1024 square |
| `hidream_o1` | HiDream O1 FP8 | Yes | Yes | — | 40 steps, CFG 5, 2048 square |

The mode selector only offers bundled capabilities for the selected model. Changing models
resets sampling and size controls to that model's defaults. Generation output sizes are
editable; edit sizes come from the region policy above. Qwen scales the padded source to its
preferred resolution before encoding, then restores the uploaded dimensions in the workflow.
HiDream's generation selector offers its supported training resolutions; editing uses 2048 square.
Instruction edit presets do not expose an img2img strength slider.

The SDXL preset targets the official
[SDXL 1.0 inpainting 0.1 model](https://huggingface.co/diffusers/stable-diffusion-xl-1.0-inpainting-0.1).
Its exported graph uses ComfyUI's built-in `DiffusersLoader` and `InpaintModelConditioning`,
which supply the masked-image and mask channels expected by this inpainting model. It requires
an installed Diffusers model directory, including `model_index.json`, under ComfyUI's
`models/diffusers`. **Diffusers model folder** is the directory choice exposed by that server,
not a single checkpoint filename. Open the exported workflow in ComfyUI and resolve its missing
model there. The legacy loader is marked deprecated upstream; Check Availability verifies that
both it and the selected folder are exposed by your server. No weights are bundled or installed.
The graph is a PhotoCraft adaptation of the
[official ComfyUI node interfaces](https://github.com/Comfy-Org/ComfyUI/blob/master/nodes.py).

**SDXL Inpaint ControlNet (Dreamer)** is a separate inpaint-only preset targeting
[Destitech's ControlNet Inpaint Dreamer SDXL](https://huggingface.co/destitech/controlnet-inpaint-dreamer-sdxl).
It uses a regular SDXL base checkpoint, default `sd_xl_base_1.0.safetensors`, in
`models/checkpoints`, plus the publisher's `diffusion_pytorch_model.fp16.safetensors`
in `models/controlnet`. Set **Checkpoint** and **ControlNet** independently in Setup;
renamed files and subfolder paths must match the choices exposed by your ComfyUI server.
Save the starter workflow and resolve missing models in ComfyUI, then Check Availability.
No weights are included or installed. Availability checks verify both model filenames and
all required native nodes before submission.

The original PhotoCraft graph follows the publisher's conditioning recipe: composite white
into the uploaded image through the uploaded mask, apply `ControlNetApply` to positive
conditioning at strength 1, and sample an empty latent at full denoise. The starting defaults
are 15 steps, CFG 8, `uni_pc_bh2`, normal scheduler. Both the white image and latent dimensions
track the actual padded input. The existing context crop, 1024 minimum, 2048 maximum,
padding removal and original selection layer mask apply unchanged. This is a regular SDXL
ControlNet; it does not use the AliMama-specific mask-channel node. The publisher describes
the model as an early alpha; compare results with SDXL 1.0 Inpaint rather than assuming it
will always improve quality.

**SDXL Union ProMax Inpaint (Xinsir)** is another separate inpaint-only preset for
[Xinsir's Union SDXL repository](https://huggingface.co/xinsir/controlnet-union-sdxl-1.0).
Use its **`diffusion_pytorch_model_promax.safetensors`** in `models/controlnet`;
the ordinary `diffusion_pytorch_model.safetensors` is not the inpainting variant.
It also needs a regular SDXL base checkpoint in `models/checkpoints`.
Setup exposes the independent Checkpoint/ControlNet filenames, workflow export and availability
checks, including the native `SetUnionControlNetType` node and its `repaint` choice.

The original PhotoCraft graph follows the publisher's
[inpainting workflow](https://github.com/xinsir6/ControlNetPlus/blob/main/workflow/inpaint.json):
binary generation mask, black-filled edit region, Union `repaint` conditioning on both positive
and negative prompts, `VAEEncodeForInpaint`, 25 steps, CFG 4.5, `dpm_2`/Karras,
ControlNet strength 0.75 and full denoise. It uses native nodes instead of custom mask helpers.
The original soft selection is retained for PhotoCraft's editable result mask. The graph returns
the full decoded image, leaving final masking to PhotoCraft so generated context remains
available underneath the mask. The same crop, pad/unpad, dimension limits and restored layer
placement apply. Open the exported workflow in ComfyUI to resolve dependencies manually;
PhotoCraft does not install or bundle model weights.

Qwen defaults to `Qwen-Image-Edit-2511-Lightning-8steps-V1.0-bf16.safetensors`.
The additional FP8/four-step preset uses `qwen_image_edit_2511_fp8mixed.safetensors` and
`Qwen-Image-Edit-2511-Lightning-4steps-V1.0-bf16.safetensors`, matching the files reported by
the local ComfyUI server during integration. Its four-step defaults match that LoRA.
Its GGUF preset requires `UnetLoaderGGUF` from ComfyUI-GGUF. Availability checks cover all
loaders, including the LoRA and custom GGUF node. Exporting the starter includes your filename
overrides, so the workflow you open in ComfyUI uses the same models as PhotoCraft.
Model weights are never copied from VibeWarp or installed by PhotoCraft.

The additional source templates are `image_flux2_klein_image_edit_{4b,9b}_distilled.json`,
`image_qwen_image_edit_2511_int8.json`, `image_mage_flow_edit{,_turbo}_int8.json`, and
`image_hidream_o1.json`. API/UI pairs live in each preset's bundled JSON; widget ordering is
explicit for these known graphs. Their setup exports are ordinary editable frontend graphs,
without subgraphs or VibeWarp's video/temporal processing.

## Other image models and modes

Use [Comfy Org's official templates](https://github.com/Comfy-Org/workflow_templates) for
instruction-guided editing (such as FLUX Kontext), reference-image edits,
outpainting, upscaling, restoration, or ControlNet-guided generation. These are custom workflow
use cases, not additional bundled presets. Install dependencies by opening the original workflow
in ComfyUI, then export **API format**. The UI-format graph with `nodes` and `links` is for setup;
the API-format object keyed by node ID is for execution. PhotoCraft deliberately does not guess
custom widget ordering or convert arbitrary frontend subgraphs.

Load the API JSON in Setup's custom workflow section and enter explicit semantic bindings:

```json
{
  "prompt": {"node": "6", "input": "text"},
  "image": {"node": "20", "input": "image"},
  "mask": {"node": "100", "input": "image"},
  "seed": {"node": "3", "input": "seed"},
  "denoise": {"node": "3", "input": "denoise"}
}
```

Node IDs and input names must match that particular workflow. `prompt` is required for text
generation and the bundled starters; custom image-only workflows such as upscalers can omit
it. `image` is required for img2img/inpaint and `mask` for inpaint. Optional bindings are `negative`,
`checkpoint`, `seed`, `steps`, `cfg`, `denoise`, `width`, and `height`. Unbound inputs retain
the workflow's values. API callers can omit numeric overrides to preserve custom defaults.
Use SaveImage or PreviewImage outputs. Custom outpainting and upscaling can use img2img mode,
which accepts a differently sized result and places it at the canvas origin. Adjust the canvas
and placement afterwards. Multi-reference uploads and video/audio results are not implemented.

## Commands and API

All actions are engine commands usable through CLI, MCP and the control channel:

| Command | Purpose |
|---|---|
| `comfyui.workflow {mode,preset?}` | Return starter UI/API graphs, bindings, preset catalog and provenance |
| `comfyui.workflow.save {mode,preset?,path,…}` | Atomically save the UI graph with model overrides |
| `comfyui.check {endpoint,mode,preset?,…}` | Inspect workflow nodes and model/enum choices |
| `comfyui.run {endpoint,mode,preset?,prompt,…}` | Generate or edit; return remote prompt ID and new layers/documents |

Example engine params:

```json
{"endpoint":"http://127.0.0.1:8188","mode":"img2img","preset":"flux2_klein_4b","prompt":"Turn the scene into a watercolor"}
```

Use `Session::start` or control `engine.execute` with `wait:false` for background work;
`jobs.list` and `jobs.cancel` use PhotoCraft's existing job lifecycle. `ui.inspect` exposes
`comfyui`; `ui.set {comfyui:{open:true,endpoint:"…",prompt:"…"}}` patches the window state.
Set `comfyui.tab` to `"setup"` or `"generate"` to switch tabs through the control channel.
`ui.set {comfyui:{preset:"qwen_edit_2511"}}` selects a preset and applies its defaults;
explicit sampling fields in the same patch override those defaults. `model_files` holds
per-preset filename overrides, for example `{"qwen_edit_2511":{"unet":"custom.safetensors"}}`.
Commands accept `checkpoint`, `unet`, `clip`, `vae` and `lora` filename overrides directly.
Custom workflows override preset graphs and use their explicit bindings; unbound numeric
inputs retain workflow values when API callers omit overrides.
Capability-scoped automation refuses `comfyui.workflow.save`; agents can obtain its JSON with
`comfyui.workflow` instead and save through their authorized workspace.

The integration uses ComfyUI's documented [`/object_info`, `/upload/image`, `/prompt`,
`/history/{prompt_id}`, `/view`, and `/queue` routes](https://docs.comfy.org/development/comfyui-server/comms_routes).
Model availability is checked against the loader inputs' current enum choices from
`/object_info`. This confirms filenames are discoverable, not that weights are valid or fit
in memory. ComfyUI's queue validation and execution errors are reported to PhotoCraft.

## Limits and current scope

- Native HTTP execution; the Rust web build still compiles and returns an explicit unsupported
  error for execution. Workflow JSON remains available on the web.
- sRGB 8-bit PNG transport, converted through PhotoCraft's CMS from the document's runtime
  profile/model/depth. Results are converted back to the destination profile and storage depth;
  the original layers stay intact. HDR and precision beyond the model transport are not retained
  in generated pixels. Outputs must be PNG.
- At most 1024 nodes, 8 MiB graph/discovery responses, 64 MiB uploaded/downloaded image bytes,
  16 outputs and 16 megapixels across outputs. Input images are capped at 16 megapixels and
  8192 pixels per side; starter generation dimensions are multiples of eight in 64–4096.
- Requests time out after 15 seconds; queued execution is polled every 250 ms with a ten-minute
  deadline. Cancellation discards the result locally and requests deletion of this pending
  prompt only. A prompt already running may finish in ComfyUI; PhotoCraft never calls global
  `/interrupt` because it may stop another client's work.
- Reverse-proxy URL prefixes are supported. Authentication headers, Comfy Cloud's distinct
  API, live WebSocket previews, job reconnection, and multi-reference image inputs
  are future work. Use a reachable self-hosted endpoint for this version.

Tests cover hostile parameters, graph links/cycles, missing nodes/models, history outcomes,
HTTP transfer routes with a mock server, mask polarity, and RGB/Gray/CMYK/Lab at 8/16/32-bit
depths. Live smoke tests on 2026-10-10 verified FLUX.2 Klein 4B generation and editing and
Qwen Image Edit 2511 FP8/four-step editing through PhotoCraft's CLI and the local ComfyUI API.
Other presets have graph, binding, mock-transfer and live availability checks; full inference
still needs testing once their model files are installed.
