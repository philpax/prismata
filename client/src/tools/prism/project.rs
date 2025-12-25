use bevy::prelude::*;
use image::GenericImageView;

use crate::voxel::{self, ChunkPendingChunkSpheres};

use super::{PrismRenderDepthOutput, PrismStateRendered};

pub fn plugin(app: &mut App) {
    app.add_message::<ProjectionRequest>()
        .add_message::<ProjectionComplete>()
        .add_systems(Update, process_incoming_project_requests);
}

#[derive(Clone, Message)]
pub struct ProjectionRequest {
    pub request_id: u64,
    pub render: image::DynamicImage,
    pub original_depth: (u32, u32, Vec<f32>),
    pub estimated_depth: Option<PrismRenderDepthOutput>,
    pub near_plane: f32,
    pub far_plane: f32,
    pub world: Transform,
    pub view_projection: Mat4,
    pub mask: Vec<bool>,
    pub use_estimation: bool,
    pub scale: f32,
}
impl ProjectionRequest {
    pub fn from_rendered(
        rendered: &PrismStateRendered,
        global_transform: &GlobalTransform,
    ) -> Self {
        let world = global_transform.compute_transform();
        let projection = Mat4::from_cols_array(&rendered.input.camera_projection);
        let view = Mat4::from(GlobalTransform::from(world).affine()).inverse();
        let view_projection = projection * view;

        ProjectionRequest {
            request_id: rand::random(),
            render: rendered.render.clone(),
            original_depth: rendered.input.depth_raw.clone(),
            estimated_depth: rendered.depth.clone(),
            near_plane: rendered.input.near_plane,
            far_plane: rendered.input.far_plane,
            world,
            view_projection,
            mask: rendered.mask.clone(),
            use_estimation: rendered.use_estimation,
            scale: rendered.scale,
        }
    }

    pub fn calculate_data(&self, voxel_size_meters: voxel::VoxelSizeMeters) -> Vec<voxel::Splat> {
        let ProjectionRequest {
            render,
            original_depth,
            estimated_depth,
            near_plane,
            far_plane,
            view_projection,
            world,
            mask,
            use_estimation,
            scale,
            ..
        } = self;

        let mut render = render.to_rgb8();
        fn srgb_to_linear_component(srgb: u8) -> u8 {
            let v = f32::from(srgb) / 255.0;
            let linear = if v <= 0.04045 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            };
            (linear * 255.0).round() as u8
        }
        for pixel in render.pixels_mut() {
            let r = srgb_to_linear_component(pixel[0]);
            let g = srgb_to_linear_component(pixel[1]);
            let b = srgb_to_linear_component(pixel[2]);
            *pixel = image::Rgb([r, g, b]);
        }
        let render = image::DynamicImage::from(render);
        calculate_data(
            &render,
            original_depth,
            estimated_depth.as_ref(),
            *use_estimation,
            mask,
            view_projection,
            world,
            voxel_size_meters,
            *scale,
            *near_plane,
            *far_plane,
        )
    }
}

#[derive(Clone, Message)]
pub struct ProjectionComplete {
    pub request_id: u64,
}

fn process_incoming_project_requests(
    mut chunks: ResMut<voxel::Chunks>,
    mut pending_chunk_spheres: Query<&mut ChunkPendingChunkSpheres>,
    mut commands: Commands,
    mut projection_complete: MessageWriter<ProjectionComplete>,
    mut requests: MessageReader<ProjectionRequest>,
    voxels_per_meter: Res<voxel::VoxelsPerMeter>,
    voxel_size_meters: Res<voxel::VoxelSizeMeters>,
) {
    for request in requests.read() {
        let now = web_time::Instant::now();
        let splats = request.calculate_data(*voxel_size_meters);
        info!("Calculating splat data took {:?}", now.elapsed());

        voxel::bulk_insert_splats(
            &mut chunks,
            &mut pending_chunk_spheres,
            *voxels_per_meter,
            &mut commands,
            &splats,
        );

        projection_complete.write(ProjectionComplete {
            request_id: request.request_id,
        });
    }
}

pub fn calculate_data(
    image: &image::DynamicImage,
    original_depth: &(u32, u32, Vec<f32>),
    estimated_depth: Option<&PrismRenderDepthOutput>,
    use_estimation: bool,
    mask: &[bool],
    view_projection: &Mat4,
    world: &Transform,
    voxel_size_meters: voxel::VoxelSizeMeters,
    scale: f32,
    near_plane: f32,
    far_plane: f32,
) -> Vec<voxel::Splat> {
    let (width, height) = image.dimensions();
    let mut instances = Vec::with_capacity((width * height) as usize);

    let inverse_view_projection = view_projection.inverse();

    let (original_width, original_height) = (original_depth.0, original_depth.1);
    let width_scale = original_width as f32 / width as f32;
    let height_scale = original_height as f32 / height as f32;

    // TODO: do this a bit more intelligently
    let max_repaint_distance = 1_000.0 * voxel_size_meters.0;
    let max_repaint_distance_sqr = max_repaint_distance.powi(2);

    let translation = world.translation;
    let mut min_metric_depth = f32::INFINITY;
    let mut max_metric_depth = f32::NEG_INFINITY;
    for y in 0..height {
        for x in 0..width {
            let scaled_i = ((y as f32 * height_scale) * original_width as f32
                + x as f32 * width_scale) as usize;

            if !mask[scaled_i] {
                continue;
            }

            let nx = x as f32 / width as f32;
            let ny = y as f32 / height as f32;

            let ndc_x = nx * 2.0 - 1.0;
            let ndc_y = -(ny * 2.0 - 1.0);
            let ndc_z = if let Some(depth) = estimated_depth.filter(|_| use_estimation) {
                // Use estimated depth and focal length for unprojection
                let depth_value = depth.metric_depth[scaled_i] * scale;

                min_metric_depth = min_metric_depth.min(depth_value);
                max_metric_depth = max_metric_depth.max(depth_value);

                depth_value - near_plane / (far_plane - near_plane)
            } else {
                // This is *extremely* suspect. This should be z*2-1 (i.e. 0..1 -> -1..1), right?
                // But the unprojection only works if it's 0..1. Not sure why.
                original_depth.2[scaled_i]
            };

            let ndc = Vec4::new(ndc_x, ndc_y, ndc_z, 1.0);
            let world_space = inverse_view_projection * ndc;
            let world_space = world_space.truncate() / world_space.w;

            if world_space.distance_squared(translation) > max_repaint_distance_sqr {
                continue;
            }

            let pixel = image.get_pixel(x, y);
            instances.push(voxel::Splat {
                position: world_space,
                color: [pixel[0], pixel[1], pixel[2]]
                    .map(|c| c as f32 / 255.0)
                    .into(),
                // TODO: Does this make sense?
                size: 2.0 * voxel_size_meters.0,
            });
        }
    }

    info!(
        "Min metric depth: {}, max metric depth: {}",
        min_metric_depth, max_metric_depth
    );

    instances
}
