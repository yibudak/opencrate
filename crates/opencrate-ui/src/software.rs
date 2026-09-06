//! CPU painting with disposable frame buffers and bounded texture backing.

use egui::{epaint::ImageDelta, ImageData, TextureId, TextureOptions, TexturesDelta};
use egui_software_backend::{BufferMutRef, ColorFieldOrder, EguiSoftwareRender};
use std::{collections::HashMap, error::Error, num::NonZeroU32, sync::Arc};
use winit::window::Window;

/// Keep the current images so the renderer can be discarded while in the tray.
/// Partial atlas updates replace pixels, rather than accumulating a delta log.
#[derive(Default)]
pub struct Textures(HashMap<TextureId, (ImageData, TextureOptions)>);

impl Textures {
    pub fn apply(&mut self, delta: &TexturesDelta) {
        for (id, change) in &delta.set {
            if let Some([x, y]) = change.pos {
                let (ImageData::Color(image), options) =
                    self.0.get_mut(id).expect("existing texture");
                let ImageData::Color(patch) = &change.image;
                let image = Arc::make_mut(image);
                assert!(x + patch.width() <= image.width());
                assert!(y + patch.height() <= image.height());
                for row in 0..patch.height() {
                    let start = (y + row) * image.width() + x;
                    image.pixels[start..start + patch.width()].copy_from_slice(
                        &patch.pixels[row * patch.width()..(row + 1) * patch.width()],
                    );
                }
                *options = change.options;
            } else {
                self.0.insert(*id, (change.image.clone(), change.options));
            }
        }
    }

    pub fn free(&mut self, delta: &TexturesDelta) {
        for id in &delta.free {
            self.0.remove(id);
        }
    }

    fn full_delta(&self) -> TexturesDelta {
        TexturesDelta {
            set: self
                .0
                .iter()
                .map(|(id, (image, options))| (*id, ImageDelta::full(image.clone(), *options)))
                .collect(),
            free: Vec::new(),
        }
    }
}

pub struct Painter {
    // Drop the surface before its window. Softbuffer presents through GDI on Windows.
    surface: softbuffer::Surface<Arc<Window>, Arc<Window>>,
    renderer: EguiSoftwareRender,
    first_frame: bool,
}

impl Painter {
    pub fn new(window: Arc<Window>) -> Result<Self, Box<dyn Error>> {
        let context = softbuffer::Context::new(window.clone())?;
        Ok(Self {
            surface: softbuffer::Surface::new(&context, window)?,
            renderer: EguiSoftwareRender::new(ColorFieldOrder::Bgra),
            first_frame: true,
        })
    }

    pub fn paint(
        &mut self,
        size: [NonZeroU32; 2],
        primitives: &[egui::ClippedPrimitive],
        delta: &TexturesDelta,
        textures: &Textures,
        scale: f32,
    ) -> Result<(), Box<dyn Error>> {
        self.surface.resize(size[0], size[1])?;
        let mut buffer = self.surface.buffer_mut()?;
        buffer.fill(0x0011_1317);
        let initial;
        let delta = if self.first_frame {
            initial = TexturesDelta {
                free: delta.free.clone(),
                ..textures.full_delta()
            };
            &initial
        } else {
            delta
        };
        self.renderer.render(
            &mut BufferMutRef::new(
                bytemuck::cast_slice_mut(&mut buffer),
                size[0].get() as usize,
                size[1].get() as usize,
            ),
            primitives,
            delta,
            scale,
        );
        #[cfg(feature = "diagnostics")]
        crate::diagnostics::painted(&buffer, size.map(NonZeroU32::get));
        buffer.present()?;
        self.first_frame = false;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Color32, ColorImage};

    #[test]
    fn partial_atlas_updates_survive_renderer_recreation_and_frees() {
        let id = TextureId::Managed(0);
        let mut textures = Textures::default();
        textures.apply(&TexturesDelta {
            set: vec![(
                id,
                ImageDelta::full(
                    ColorImage::filled([4, 3], Color32::BLACK),
                    TextureOptions::LINEAR,
                ),
            )],
            ..Default::default()
        });
        for _ in 0..1000 {
            textures.apply(&TexturesDelta {
                set: vec![(
                    id,
                    ImageDelta::partial(
                        [1, 1],
                        ColorImage::filled([2, 1], Color32::WHITE),
                        TextureOptions::NEAREST,
                    ),
                )],
                ..Default::default()
            });
        }
        let full = textures.full_delta();
        assert_eq!(full.set.len(), 1);
        assert_eq!(full.set[0].1.pos, None);
        assert_eq!(full.set[0].1.options, TextureOptions::NEAREST);
        let ImageData::Color(image) = &full.set[0].1.image;
        assert_eq!(image.pixels.len(), 12);
        assert_eq!(image.pixels[4], Color32::BLACK);
        assert_eq!(&image.pixels[5..7], &[Color32::WHITE; 2]);
        assert_eq!(image.pixels[7], Color32::BLACK);
        textures.free(&TexturesDelta {
            free: vec![id],
            ..Default::default()
        });
        assert!(textures.full_delta().set.is_empty());
    }
}
