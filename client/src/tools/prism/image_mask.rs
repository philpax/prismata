// TODO: optimise a lot of this, lots of unnecessary allocation!

use std::{
    collections::VecDeque,
    hash::{Hash, Hasher},
};

use bevy::math::Vec4;
use bevy_egui::egui::{self, Vec2};
use image::{DynamicImage, GenericImageView, Rgb, RgbImage};

pub struct ImageMask<'a> {
    pub sized_texture: egui::load::SizedTexture,
    pub size: egui::ImageSize,
    pub mask: &'a mut [bool],
    pub mask_texture: egui::TextureHandle,
    pub wand_source_image: &'a DynamicImage,
}
impl<'a> ImageMask<'a> {
    pub fn new(
        sized_texture: egui::load::SizedTexture,
        max_size: Vec2,
        mask: &'a mut [bool],
        mask_texture: egui::TextureHandle,
        wand_source_image: &'a DynamicImage,
    ) -> Self {
        assert_eq!(
            (sized_texture.size.x * sized_texture.size.y) as usize,
            mask.len()
        );
        Self {
            sized_texture,
            size: egui::ImageSize {
                maintain_aspect_ratio: true,
                max_size,
                fit: egui::ImageFit::Exact(sized_texture.size),
            },
            mask,
            mask_texture,
            wand_source_image,
        }
    }
}
#[derive(Clone)]
struct ImageMaskState {
    tool: ImageMaskTool,
    radius: u32,
    tolerance: u8,
    mask_hash: u64,
    mask_changed: bool,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum ImageMaskTool {
    Add,
    Sub,
    Wand,
}
impl Default for ImageMaskState {
    fn default() -> Self {
        Self {
            tool: ImageMaskTool::Wand,
            radius: 25,
            tolerance: 20,
            mask_hash: 0,
            mask_changed: false,
        }
    }
}
impl<'a> egui::Widget for ImageMask<'a> {
    fn ui(mut self, ui: &mut egui::Ui) -> egui::Response {
        ui.push_id("image_mask", |ui| {
            ui.vertical(|ui| {
                let id = ui.id();
                let response = self.image(ui, id);
                ui.horizontal(|ui| {
                    let mut state = ui.memory_mut(|m| {
                        m.data.get_temp_mut_or_default::<ImageMaskState>(id).clone()
                    });

                    if ui
                        .add(
                            egui::Button::new(format!(
                                "{} Add",
                                egui_phosphor::regular::PLUS_CIRCLE
                            ))
                            .selected(state.tool == ImageMaskTool::Add),
                        )
                        .clicked()
                    {
                        state.tool = ImageMaskTool::Add;
                    } else if ui
                        .add(
                            egui::Button::new(format!(
                                "{} Sub",
                                egui_phosphor::regular::MINUS_CIRCLE
                            ))
                            .selected(state.tool == ImageMaskTool::Sub),
                        )
                        .clicked()
                    {
                        state.tool = ImageMaskTool::Sub;
                    } else if ui
                        .add(
                            egui::Button::new(format!(
                                "{} Wand",
                                egui_phosphor::regular::MAGIC_WAND
                            ))
                            .selected(state.tool == ImageMaskTool::Wand),
                        )
                        .clicked()
                    {
                        state.tool = ImageMaskTool::Wand;
                    } else if ui
                        .add(egui::Button::new(format!(
                            "{} Clear",
                            egui_phosphor::regular::SELECTION_SLASH
                        )))
                        .clicked()
                    {
                        self.mask.fill(false);
                    } else if ui
                        .add(egui::Button::new(format!(
                            "{} Invert",
                            egui_phosphor::regular::SELECTION_INVERSE
                        )))
                        .clicked()
                    {
                        for set in self.mask.iter_mut() {
                            *set = !*set;
                        }
                    } else if ui
                        .add(egui::Button::new(format!(
                            "{} Fill",
                            egui_phosphor::regular::SELECTION_ALL
                        )))
                        .clicked()
                    {
                        self.mask.fill(true);
                    }

                    match state.tool {
                        ImageMaskTool::Add | ImageMaskTool::Sub => {
                            let min = 1;
                            let max = 250;

                            state.radius = (state.radius as i32
                                + ui.input(|i| i.smooth_scroll_delta.y as i32))
                            .clamp(min as i32, max as i32)
                                as u32;

                            ui.label("Radius:");
                            ui.add(egui::Slider::new(&mut state.radius, min..=max));
                        }
                        ImageMaskTool::Wand => {
                            let min = 1;
                            let max = 100;
                            ui.label("Tolerance:");
                            ui.add(egui::Slider::new(&mut state.tolerance, min..=max));
                        }
                    }

                    let new_hash = {
                        let mut hasher = std::collections::hash_map::DefaultHasher::new();
                        self.mask.hash(&mut hasher);
                        hasher.finish()
                    };
                    state.mask_changed = state.mask_hash != new_hash;
                    state.mask_hash = new_hash;

                    ui.memory_mut(|m| {
                        m.data.insert_temp(id, state);
                    });
                });
                response
            })
            .inner
        })
        .inner
    }
}
impl<'a> ImageMask<'a> {
    fn image(&mut self, ui: &mut egui::Ui, id: egui::Id) -> egui::Response {
        let image_size = self.sized_texture.size;
        let ui_size = self.size.calc_size(ui.available_size(), image_size);

        let (rect, mut response) = ui.allocate_exact_size(ui_size, egui::Sense::drag());

        let to_screen = egui::emath::RectTransform::from_to(
            egui::Rect::from_min_size(egui::Pos2::ZERO, rect.square_proportions()),
            rect,
        );
        let from_screen = to_screen.inverse();

        let state = ui.memory(|r| r.data.get_temp::<ImageMaskState>(id).unwrap_or_default());
        let r = state.radius as isize;

        let mask_changed = state.mask_changed;
        if let Some(pointer_pos) = response.interact_pointer_pos() {
            let canvas_pos =
                (from_screen * pointer_pos).clamp(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));

            let x = (canvas_pos.x * image_size.x) as usize;
            let y = (canvas_pos.y * image_size.y) as usize;

            match state.tool {
                ImageMaskTool::Add | ImageMaskTool::Sub => {
                    for dy in -r..=r {
                        for dx in -r..=r {
                            let x = x as isize + dx;
                            let y = y as isize + dy;
                            if !(x >= 0
                                && x < image_size.x as isize
                                && y >= 0
                                && y < image_size.y as isize)
                            {
                                continue;
                            }

                            if dx * dx + dy * dy <= r * r {
                                self.mask[(y * image_size.x as isize + x) as usize] =
                                    state.tool == ImageMaskTool::Add;
                            }
                        }
                    }
                }
                ImageMaskTool::Wand => {
                    magic_wand(self.mask, (x, y), self.wand_source_image, state.tolerance);
                }
            }
        }

        if mask_changed {
            let mut buf = vec![0u8; self.mask.len() * 4];
            for (idx, set) in self.mask.iter().copied().enumerate() {
                let color = if set {
                    egui::Color32::WHITE
                } else {
                    egui::Color32::TRANSPARENT
                };
                buf[idx * 4..(idx + 1) * 4].copy_from_slice(&color.to_array());
            }
            self.mask_texture.set(
                egui::ColorImage::from_rgba_unmultiplied(
                    [image_size.x as usize, image_size.y as usize],
                    &buf,
                ),
                egui::TextureOptions::default(),
            );
            response.mark_changed();
        }

        if ui.is_rect_visible(rect) {
            let mut mesh = egui::Mesh::with_texture(self.sized_texture.id);
            let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
            mesh.add_rect_with_uv(rect, uv, egui::Color32::WHITE);
            ui.painter().add(egui::Shape::mesh(mesh));

            let mut mesh = egui::Mesh::with_texture(self.mask_texture.id());
            mesh.add_rect_with_uv(
                rect,
                uv,
                egui::Color32::from_rgba_unmultiplied(255, 255, 255, 128),
            );
            ui.painter().add(egui::Shape::mesh(mesh));
        }

        if matches!(state.tool, ImageMaskTool::Add | ImageMaskTool::Sub) {
            if let Some(hover_pos) = ui
                .input(|i| i.pointer.latest_pos())
                .filter(|p| rect.contains(*p))
            {
                let mut color = if state.tool == ImageMaskTool::Add {
                    egui::Color32::GREEN
                } else {
                    egui::Color32::RED
                };
                color[3] = 128;

                let egui_size = ui.ctx().content_rect().size().min_elem();
                let ui_size = rect.size().min_elem();
                let scale = ui_size / egui_size;

                ui.painter()
                    .with_clip_rect(rect)
                    .circle_filled(hover_pos, r as f32 * scale, color);
            }
        }

        response
    }
}

pub fn mask_to_image(mask: &[bool], size_x: usize, size_y: usize) -> RgbImage {
    RgbImage::from_fn(size_x as u32, size_y as u32, |x, y| {
        let set = mask[y as usize * size_x + x as usize];
        if set {
            Rgb([255, 255, 255])
        } else {
            Rgb([0, 0, 0])
        }
    })
}

pub fn magic_wand(
    mask: &mut [bool],
    seed: (usize, usize),
    wand_source_image: &DynamicImage,
    tolerance: u8,
) {
    let (width, height) = wand_source_image.dimensions();
    let seed_pixel = wand_source_image.get_pixel(seed.0 as u32, seed.1 as u32).0;

    let mut queue = VecDeque::new();
    queue.push_back(seed);

    while let Some((x, y)) = queue.pop_front() {
        let index = y * width as usize + x;

        if mask[index] {
            continue; // Skip if already processed
        }

        let current_pixel = wand_source_image.get_pixel(x as u32, y as u32).0;
        if color_difference(seed_pixel, current_pixel).abs() as u8 <= tolerance {
            mask[index] = true;

            // Check and enqueue neighbors
            let neighbors = [
                (x.wrapping_sub(1), y), // Left
                (x + 1, y),             // Right
                (x, y.wrapping_sub(1)), // Up
                (x, y + 1),             // Down
            ];

            for (nx, ny) in neighbors.iter() {
                if *nx < width as usize && *ny < height as usize {
                    queue.push_back((*nx, *ny));
                }
            }
        }
    }
}

fn color_difference(a: [u8; 4], b: [u8; 4]) -> f32 {
    let a = Vec4::from_array(a.map(|c| c as f32)).truncate();
    let b = Vec4::from_array(b.map(|c| c as f32)).truncate();

    a.distance(b)
}
