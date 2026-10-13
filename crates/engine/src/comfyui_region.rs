//! Reversible document-to-model geometry. Coordinates always refer to the captured document.
#[cfg(not(target_arch = "wasm32"))]
use super::error;
use super::{bad, number, preset};
use crate::Result;
use photocraft_doc::Document;
use photocraft_geom::Rect;
use serde::Serialize;
use serde_json::Value;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Region {
    pub crop: Rect,
    pub inner_width: u32,
    pub inner_height: u32,
    pub width: u32,
    pub height: u32,
    pub pad_left: u32,
    pub pad_top: u32,
}

impl Region {
    pub fn plan(doc: &Document, p: &Value) -> Result<Self> {
        let model = preset(p)?;
        let margin = number(p, "context", 128.0, 0.0, 1024.0, true)? as i32;
        let limit = number(p, "maxSize", 2048.0, 1024.0, 2048.0, true)? as u32;
        let minimum = if model.id == "hidream_o1" { 2048 } else { 1024 };
        if limit < minimum {
            return Err(bad("maximum processing size is below this model's minimum"));
        }
        let bounds = doc.bounds();
        let area = if let Some(selection) = &doc.selection {
            if selection.default_pixel().first().copied().unwrap_or(0.0) > 0.0 { bounds } else { selection.content_bounds().intersect(&bounds) }
        } else {
            bounds
        };
        if area.is_empty() {
            return Err(bad("the selection has no pixels inside the canvas"));
        }
        let crop = Rect::new(area.x0.saturating_sub(margin), area.y0.saturating_sub(margin), area.x1.saturating_add(margin), area.y1.saturating_add(margin))
            .intersect(&bounds);
        // Restoring native-depth results is bounded independently of inference resolution.
        if crop.width() > 32768
            || crop.height() > 32768
            || u64::from(crop.width()) * u64::from(crop.height()) * doc.pixel_format().bytes_per_pixel() as u64 > 512 * 1024 * 1024
        {
            return Err(bad("edit region exceeds the 512 MiB restoration budget or 32768 pixels per side; select a smaller area"));
        }
        let limit = limit / 16 * 16;
        let scale = (f64::from(limit) / f64::from(crop.width().max(crop.height()))).min(1.0);
        let inner_width = (f64::from(crop.width()) * scale).round().max(1.0) as u32;
        let inner_height = (f64::from(crop.height()) * scale).round().max(1.0) as u32;
        let width = if model.id == "hidream_o1" { 2048 } else { inner_width.max(minimum).div_ceil(16) * 16 };
        let height = if model.id == "hidream_o1" { 2048 } else { inner_height.max(minimum).div_ceil(16) * 16 };
        Ok(Self { crop, inner_width, inner_height, width, height, pad_left: (width - inner_width) / 2, pad_top: (height - inner_height) / 2 })
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn input_png(&self, doc: &Document, mask: bool) -> Result<Vec<u8>> {
        use photocraft_codecs::{ChannelLayout, EncodeOptions, Format, Image, SampleType};
        use photocraft_color::PixelFormat;
        let inner = Rect::from_xywh(0, 0, self.inner_width, self.inner_height);
        let bytes = if mask {
            let selection = doc.selection.as_ref().ok_or_else(|| bad("inpaint requires a selection"))?;
            let cropped = photocraft_algo::resample::crop_surface(selection, self.crop).translated(-self.crop.x0, -self.crop.y0, self.crop);
            photocraft_algo::resample::resize_surface_in_canvas(
                &cropped,
                f64::from(self.inner_width) / f64::from(self.crop.width()),
                f64::from(self.inner_height) / f64::from(self.crop.height()),
                photocraft_algo::resample::Resample::Bilinear,
                Rect::from_xywh(0, 0, self.crop.width(), self.crop.height()),
            )
            .convert(PixelFormat::GRAY8)
            .to_interleaved(inner)
        } else {
            let buffer = photocraft_compose::render_reduced_rect(doc, self.crop, self.inner_width, self.inner_height);
            let mut surface = photocraft_raster::Surface::new(PixelFormat::RGBA32F);
            let values: Vec<f32> = buffer.px.into_iter().flatten().collect();
            surface.write_region(inner, &values);
            let transform = photocraft_cms::Transform::new(
                &crate::color_cmds::composite_profile(doc),
                photocraft_cms::Builtin::Srgb.profile(),
                photocraft_cms::Intent::RelativeColorimetric,
                true,
            )
            .map_err(error)?;
            crate::color_cmds::convert_surface(&surface, photocraft_color::ColorMode::Rgb, PixelFormat::RGBA32F, &transform)
                .convert(PixelFormat::RGBA8)
                .to_interleaved(inner)
        };
        let channels = if mask { 1 } else { 4 };
        let mut padded = vec![0; self.width as usize * self.height as usize * channels];
        for y in 0..self.height {
            for x in 0..self.width {
                let inside = x >= self.pad_left && x < self.pad_left + self.inner_width && y >= self.pad_top && y < self.pad_top + self.inner_height;
                if mask && !inside {
                    continue;
                }
                // Image padding repeats the nearest context pixel; mask padding always preserves.
                let sx = x.saturating_sub(self.pad_left).min(self.inner_width - 1);
                let sy = y.saturating_sub(self.pad_top).min(self.inner_height - 1);
                let source = (sy as usize * self.inner_width as usize + sx as usize) * channels;
                let target = (y as usize * self.width as usize + x as usize) * channels;
                let src = bytes.get(source..source + channels).ok_or_else(|| error("invalid input raster length"))?;
                let dst = padded.get_mut(target..target + channels).ok_or_else(|| error("invalid padded raster length"))?;
                dst.copy_from_slice(src);
            }
        }
        let mut image =
            Image::from_raw(self.width, self.height, if mask { ChannelLayout::Gray } else { ChannelLayout::Rgba }, SampleType::U8, padded).map_err(error)?;
        if !mask {
            image = image.with_icc(Some(photocraft_cms::Builtin::Srgb.profile().to_bytes().to_vec()));
        }
        photocraft_codecs::encode(&image, Format::Png, &EncodeOptions::default()).map_err(error)
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn restore(&self, doc: &mut Document) -> Result<()> {
        if u64::from(doc.size.width) * u64::from(self.height) != u64::from(doc.size.height) * u64::from(self.width) {
            return Err(bad("workflow returned a different aspect ratio from the processing region; bind its output dimensions to the uploaded input"));
        }
        super::fit_selected_output(doc, photocraft_doc::Size::new(self.width, self.height))?;
        let surface = doc.layers.first_mut().and_then(|l| l.surface_mut()).ok_or_else(|| error("ComfyUI output has no raster pixels"))?;
        let keep = Rect::from_xywh(self.pad_left as i32, self.pad_top as i32, self.inner_width, self.inner_height);
        *surface = photocraft_algo::resample::crop_surface(surface, keep).translated(-(self.pad_left as i32), -(self.pad_top as i32), keep);
        *surface = photocraft_algo::resample::resize_surface_in_canvas(
            surface,
            f64::from(self.crop.width()) / f64::from(self.inner_width),
            f64::from(self.crop.height()) / f64::from(self.inner_height),
            photocraft_algo::resample::Resample::Lanczos,
            Rect::from_xywh(0, 0, self.inner_width, self.inner_height),
        );
        *surface = photocraft_algo::resample::crop_surface(surface, Rect::from_xywh(0, 0, self.crop.width(), self.crop.height()));
        // Keep temporary document coordinates at zero until ICC conversion has finished.
        doc.size = photocraft_doc::Size::new(self.crop.width(), self.crop.height());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_doc::{ColorMode, SampleType, Size};
    use serde_json::json;

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    #[ignore = "manual release benchmark of 24 MP ComfyUI transport and restoration"]
    fn benchmark_24_megapixel_transport() {
        use photocraft_color::{Color, PixelFormat};
        let mut doc = Document::with_background("benchmark", Size::new(6000, 4000), ColorMode::Rgb, SampleType::U8, Color::WHITE);
        doc.layers.first_mut().unwrap().surface_mut().unwrap().fill_rect(Rect::from_xywh(3000, 2000, 256, 128), &[0.0, 0.0, 1.0, 1.0]);
        for selected in [false, true] {
            if selected {
                let mut mask = photocraft_raster::Surface::new(PixelFormat::GRAY8);
                mask.fill_rect(Rect::from_xywh(3000, 2000, 256, 128), &[1.0]);
                doc.selection = Some(mask);
            }
            let r = Region::plan(&doc, &json!({"preset":"flux2_klein_4b"})).unwrap();
            let start = std::time::Instant::now();
            let png = r.input_png(&doc, false).unwrap();
            let upload_ms = start.elapsed().as_millis();
            let mut result = photocraft_io::import("input.png", &png).unwrap().document;
            let start = std::time::Instant::now();
            r.restore(&mut result).unwrap();
            println!("24 MP selected={selected}: prepare={upload_ms}ms restore={}ms processing={}x{}", start.elapsed().as_millis(), r.width, r.height);
        }
    }

    #[test]
    fn geometry_pads_without_upscaling_and_downscales_proportionally() {
        let mut doc = Document::new("test", Size::new(6000, 4000), ColorMode::Rgb, SampleType::U8);
        let mut mask = photocraft_raster::Surface::new(photocraft_color::PixelFormat::GRAY8);
        mask.fill_rect(Rect::from_xywh(100, 200, 200, 100), &[1.0]);
        doc.selection = Some(mask);
        let r = Region::plan(&doc, &json!({"preset":"sdxl_inpaint","context":128})).unwrap();
        assert_eq!(r.crop, Rect::new(0, 72, 428, 428));
        assert_eq!((r.inner_width, r.inner_height, r.width, r.height), (428, 356, 1024, 1024));
        doc.selection = None;
        let r = Region::plan(&doc, &json!({"preset":"flux2_klein_4b"})).unwrap();
        assert_eq!((r.inner_width, r.inner_height), (2048, 1365));
        assert_eq!((r.width, r.height), (2048, 1376));
        for p in
            [json!({"context":-1}), json!({"context":1.5}), json!({"maxSize":4096}), json!({"maxSize":"bad"}), json!({"preset":"hidream_o1","maxSize":1024})]
        {
            assert!(Region::plan(&doc, &p).is_err());
        }
    }

    #[test]
    fn empty_and_hostile_document_sizes_fail_without_allocation() {
        let mut doc = Document::new("test", Size::new(64, 64), ColorMode::Rgb, SampleType::F32);
        doc.selection = Some(photocraft_raster::Surface::new(photocraft_color::PixelFormat::GRAY8));
        assert!(Region::plan(&doc, &serde_json::json!({})).is_err());
        doc.selection = None;
        for size in [Size::new(0, 0), Size::new(i32::MAX as u32, i32::MAX as u32), Size::new(u32::MAX, u32::MAX)] {
            doc.size = size;
            assert!(Region::plan(&doc, &serde_json::json!({})).is_err());
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn proportional_reduction_restores_non_square_content_without_padding() {
        use photocraft_color::Color;
        let doc = Document::new("test", Size::new(3200, 800), ColorMode::Rgb, SampleType::U8);
        let r = Region::plan(&doc, &serde_json::json!({"maxSize":1024})).unwrap();
        assert_eq!((r.inner_width, r.inner_height, r.pad_top), (1024, 256, 384));
        let mut output = Document::with_background("output", Size::new(1024, 1024), ColorMode::Rgb, SampleType::U8, Color::WHITE);
        let surface = output.layers.first_mut().unwrap().surface_mut().unwrap();
        surface.fill_rect(Rect::from_xywh(0, 384, 512, 256), &[1.0, 0.0, 0.0, 1.0]);
        surface.fill_rect(Rect::from_xywh(512, 384, 512, 256), &[0.0, 0.0, 1.0, 1.0]);
        r.restore(&mut output).unwrap();
        assert_eq!(output.size, doc.size);
        let surface = output.layers.first().unwrap().surface().unwrap();
        assert_eq!(surface.pixel(4, 4), vec![1.0, 0.0, 0.0, 1.0]);
        assert_eq!(surface.pixel(3195, 795), vec![0.0, 0.0, 1.0, 1.0]);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn image_padding_and_mask_padding_are_removed_at_all_depths() {
        use photocraft_color::{Color, PixelFormat};
        for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let mut doc = Document::with_background("test", Size::new(64, 48), ColorMode::Rgb, depth, Color::WHITE);
            let mut mask = photocraft_raster::Surface::new(PixelFormat::GRAY8);
            mask.fill_rect(Rect::from_xywh(20, 10, 8, 6), &[1.0]);
            doc.selection = Some(mask);
            let r = Region::plan(&doc, &json!({"context":4})).unwrap();
            assert_eq!(r.crop, Rect::new(16, 6, 32, 20));
            let mask = photocraft_codecs::decode(&r.input_png(&doc, true).unwrap()).unwrap();
            assert_eq!(mask.data()[0], 0);
            assert_eq!(mask.data()[((r.pad_top + 5) * r.width + r.pad_left + 5) as usize], 255);
            let image = photocraft_codecs::decode(&r.input_png(&doc, false).unwrap()).unwrap();
            assert_eq!(image.data()[3], 255);
            let mut output = Document::with_background("output", Size::new(1024, 1024), ColorMode::Rgb, depth, Color::WHITE);
            output
                .layers
                .first_mut()
                .unwrap()
                .surface_mut()
                .unwrap()
                .fill_rect(Rect::from_xywh(r.pad_left as i32, r.pad_top as i32, r.inner_width, r.inner_height), &[1.0, 0.0, 0.0, 1.0]);
            r.restore(&mut output).unwrap();
            assert_eq!(output.size, Size::new(16, 14));
            assert_eq!(output.layers.first().unwrap().surface().unwrap().pixel(0, 0), vec![1.0, 0.0, 0.0, 1.0]);
            output.size = Size::new(32, 16);
            assert!(r.restore(&mut output).is_err());
        }
    }
}
