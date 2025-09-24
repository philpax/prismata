use serde::{Deserialize, Serialize};
use web_time::Duration;

use bevy::{
    core_pipeline::{
        prepass::DepthPrepass,
        tonemapping::{DebandDither, Tonemapping},
    },
    prelude::*,
    render::{
        camera::{CameraProjection, Exposure, RenderTarget},
        extract_component::{ExtractComponent, ExtractComponentPlugin},
        extract_resource::{ExtractResource, ExtractResourcePlugin},
        render_resource::{
            Extent3d, TextureDescriptor, TextureDimension, TextureFormat, TextureUsages,
        },
        view::{ColorGrading, RenderLayers},
        RenderApp,
    },
};
use bevy_egui::{egui, EguiUserTextures};
use bevy_mod_reqwest::{BevyReqwest, ReqwestErrorEvent, ReqwestResponseEvent};
use image::{EncodableLayout, GenericImageView};
use prismata_protocol::tools::prism::{
    decode_image_from_base64, encode_image_to_base64, encode_image_to_png, EncodedPngImage,
    RenderInput, RenderOutput, RenderOutputResult,
};
use project::{ProjectionComplete, ProjectionRequest};

use crate::{
    camera,
    file_picker::{WriteHandler, WritePlugin},
    rendering,
    ui::Toasts,
    HttpEndpoints,
};

use super::{is_active, is_in_use, ActiveTool, Tool};

mod image_mask;
use image_mask::{mask_to_image, ImageMask};

mod project;
mod project_preview;
mod render_app;

pub fn plugin(app: &mut App) {
    app.add_plugins(PrismPlugin);
}

struct PrismPlugin;
impl Plugin for PrismPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(PrismRenderSize(UVec2::new(1024, 1024)))
            .insert_resource(PrismPaintSettings::default())
            .add_systems(Startup, startup)
            .add_systems(
                Update,
                // Note that any systems that rely on data being received should run unconditionally
                // (i.e. without `is_selected` or `is_in_use`) to ensure that their data is
                // processed as soon as it is available.
                (
                    spawn_prism.run_if(
                        resource_exists::<PrismRenderSize>
                            .and_then(not(any_with_component::<PrismMainCamera>)),
                    ),
                    activate_camera,
                    sync_game_camera_and_prism_camera
                        .after(camera::update_camera)
                        .run_if(is_active(Tool::Prism)),
                    on_use.run_if(is_in_use(Tool::Prism, Some(Duration::from_millis(50)))),
                    update_state_from_events.run_if(
                        resource_exists::<PrismRenderSize>.and_then(resource_exists::<PrismState>),
                    ),
                    ui.run_if(is_active(Tool::Prism)),
                )
                    .chain(),
            )
            .add_plugins((
                project::plugin,
                project_preview::plugin,
                WritePlugin::new(ExportImageHandler),
                WritePlugin::new(ExportDepthHandler),
                WritePlugin::new(ExportRenderedImageHandler),
                WritePlugin::new(ExportRenderedDepthHandler),
            ))
            .add_event::<PrismError>()
            .add_event::<PrismRenderOutput>()
            // TODO(Bevy 0.15): Move this to a component on the Prism camera once
            // <https://github.com/bevyengine/bevy/pull/14273> is available on main.
            //
            // MSAA must be turned off to make the copy-to-buffer possible. This is
            // only necessary for the Prism camera, so this being global is a temporary
            // solution.
            .insert_resource(Msaa::Off);
    }

    fn finish(&self, app: &mut App) {
        app.add_plugins((
            ExtractComponentPlugin::<PrismMainCamera>::default(),
            ExtractComponentPlugin::<PrismMaskCamera>::default(),
            ExtractResourcePlugin::<PrismPostProcessRender>::default(),
            ExtractResourcePlugin::<PrismRenderSize>::default(),
        ))
        .get_sub_app_mut(RenderApp)
        .unwrap()
        .add_plugins(render_app::plugin);
    }
}

#[derive(Resource, Debug, Deserialize, Serialize, Clone, PartialEq)]
#[serde(default)]
pub struct PrismPaintSettings {
    pub prompt: String,
    pub seed: i64,
    pub repaint_amount: f32,
    pub sampler_steps: u8,
    pub sampler_eta: f32,
    pub depth_controlnet_strength: f32,
}
impl Default for PrismPaintSettings {
    fn default() -> Self {
        Self {
            prompt: "green mountains".into(),
            seed: 314159265,
            repaint_amount: 0.8,
            sampler_steps: 20,
            sampler_eta: 1.0,
            depth_controlnet_strength: 1.1,
        }
    }
}

#[derive(Resource, Copy, Clone, Debug, PartialEq, Eq, Deref, DerefMut, ExtractResource)]
pub struct PrismRenderSize(pub UVec2);

#[derive(Resource, Deref)]
/// The image that the Prism camera renders to.
///
/// Stored separately from the state to avoid having to recreate/re-register
/// the texture every time the state changes.
struct PrismMainImage(Handle<Image>);

#[derive(Resource, Deref)]
struct PrismMaskImage(Handle<Image>);

#[derive(Resource)]
enum PrismState {
    WaitingForCapture {
        show_mask: bool,
    },
    Capture(PrismStateCapture),
    Rendering {
        input: RenderInput,
        render_egui: Option<egui::TextureHandle>,
        start_time: web_time::Instant,
        mask: Vec<bool>,
    },
    Rendered(PrismStateRendered),
    Projecting {
        start_time: web_time::Instant,
        request_id: u64,
    },
    Error {
        message: String,
    },
}
impl Default for PrismState {
    fn default() -> Self {
        Self::WaitingForCapture { show_mask: false }
    }
}
impl PrismState {
    /// Returns `true` if the prism state is [`WaitingForCapture`].
    ///
    /// [`WaitingForCapture`]: PrismState::WaitingForCapture
    #[must_use]
    fn is_waiting_for_capture(&self) -> bool {
        matches!(self, Self::WaitingForCapture { .. })
    }

    pub fn id(&self) -> &'static str {
        match self {
            Self::WaitingForCapture { .. } => "WaitingForCapture",
            Self::Capture(_) => "Capture",
            Self::Rendering { .. } => "Rendering",
            Self::Rendered(_) => "Rendered",
            Self::Projecting { .. } => "Projecting",
            Self::Error { .. } => "Error",
        }
    }
}

struct PrismStateCapture {
    camera_position: Vec3,
    camera_rotation: Quat,
    camera_projection: Mat4,
    near_plane: f32,
    far_plane: f32,
    render: image::DynamicImage,
    render_egui: egui::TextureHandle,
    depth: NormalizedOutput,
    depth_egui: egui::TextureHandle,
    depth_data: Vec<f32>,
    mask: Vec<bool>,
    mask_texture: egui::TextureHandle,
    prompt: String,
    show_depth: bool,
}

struct PrismStateRendered {
    input: RenderInput,
    render: image::DynamicImage,
    render_egui: egui::TextureHandle,
    depth: Option<PrismRenderDepthOutput>,
    depth_egui: Option<egui::TextureHandle>,
    mask: Vec<bool>,
    mask_texture: egui::TextureHandle,
    show_depth: bool,
    preview_entity: Entity,
    use_estimation: bool,
    scale: f32,
}

#[derive(Resource)]
struct PrismPreviewMesh(Handle<Mesh>);

#[derive(Resource)]
struct PrismPreviewMaterial(Handle<StandardMaterial>);

#[derive(Event)]
struct PrismError {
    message: String,
}

#[derive(Clone, Event)]
pub struct PrismRenderOutput {
    pub render_id: u64,
    pub diffuse: image::DynamicImage,
    pub depth: Option<PrismRenderDepthOutput>,
}
impl<'a> TryFrom<&'a PrismStateRendered> for PrismRenderOutput {
    type Error = anyhow::Error;
    fn try_from(value: &'a PrismStateRendered) -> Result<Self, Self::Error> {
        Ok(Self {
            render_id: value.input.render_id,
            diffuse: value.render.clone(),
            depth: value.depth.clone(),
        })
    }
}
#[derive(Serialize, Deserialize, Clone)]
pub struct SerializablePrismRenderOutput {
    pub input: RenderInput,
    pub mask: Vec<bool>,
    pub diffuse: EncodedPngImage,
    pub depth: Option<SerializablePrismRenderDepthOutput>,
}
impl<'a> TryFrom<&'a PrismStateRendered> for SerializablePrismRenderOutput {
    type Error = anyhow::Error;
    fn try_from(value: &'a PrismStateRendered) -> Result<Self, Self::Error> {
        Ok(Self {
            input: value.input.clone(),
            mask: value.mask.clone(),
            diffuse: encode_image_to_base64(&value.render)?,
            depth: value
                .depth
                .clone()
                .map(SerializablePrismRenderDepthOutput::try_from)
                .transpose()?,
        })
    }
}
impl<'a> TryFrom<&'a SerializablePrismRenderOutput> for PrismState {
    type Error = anyhow::Error;
    fn try_from(value: &'a SerializablePrismRenderOutput) -> Result<Self, anyhow::Error> {
        Ok(Self::Rendering {
            input: value.input.clone(),
            render_egui: None,
            start_time: web_time::Instant::now(),
            mask: value.mask.clone(),
        })
    }
}
impl TryFrom<SerializablePrismRenderOutput> for PrismRenderOutput {
    type Error = anyhow::Error;
    fn try_from(value: SerializablePrismRenderOutput) -> Result<Self, Self::Error> {
        Ok(Self {
            render_id: value.input.render_id,
            diffuse: decode_image_from_base64(&value.diffuse)?,
            depth: value
                .depth
                .map(PrismRenderDepthOutput::try_from)
                .transpose()?,
        })
    }
}
#[derive(Clone)]
pub struct PrismRenderDepthOutput {
    pub metric_depth: Vec<f32>,
    pub relative_depth: image::DynamicImage,
    pub focal_length: f32,
}
#[derive(Serialize, Deserialize, Clone)]
pub struct SerializablePrismRenderDepthOutput {
    pub metric_depth: Vec<f32>,
    pub relative_depth: EncodedPngImage,
    pub focal_length: f32,
}
impl TryFrom<PrismRenderDepthOutput> for SerializablePrismRenderDepthOutput {
    type Error = anyhow::Error;
    fn try_from(value: PrismRenderDepthOutput) -> Result<Self, Self::Error> {
        Ok(Self {
            metric_depth: value.metric_depth,
            relative_depth: encode_image_to_base64(&value.relative_depth)?,
            focal_length: value.focal_length,
        })
    }
}
impl TryFrom<SerializablePrismRenderDepthOutput> for PrismRenderDepthOutput {
    type Error = anyhow::Error;
    fn try_from(value: SerializablePrismRenderDepthOutput) -> Result<Self, Self::Error> {
        Ok(Self {
            metric_depth: value.metric_depth,
            relative_depth: decode_image_from_base64(&value.relative_depth)?,
            focal_length: value.focal_length,
        })
    }
}
impl TryFrom<(String, f32)> for PrismRenderDepthOutput {
    type Error = anyhow::Error;
    fn try_from(value: (String, f32)) -> Result<Self, Self::Error> {
        let metric_depth_image = decode_image_from_base64(&value.0)?.to_rgb32f();
        let metric_depth: Vec<f32> = metric_depth_image.pixels().map(|p| p[0]).collect();
        let (mut min, mut max) = (f32::INFINITY, f32::NEG_INFINITY);
        for &v in &metric_depth {
            min = min.min(v);
            max = max.max(v);
        }

        let relative_depth = image::DynamicImage::from(image::ImageBuffer::from_fn(
            metric_depth_image.width(),
            metric_depth_image.height(),
            |x, y| {
                let v = metric_depth_image.get_pixel(x, y)[0];
                let v = (v - min) / (max - min);
                let v = (v.clamp(0.0, 1.0) * 255.0) as u8;
                image::Rgba([v, v, v, 255])
            },
        ));

        Ok(Self {
            metric_depth,
            relative_depth,
            focal_length: value.1,
        })
    }
}
impl TryFrom<RenderOutput> for PrismRenderOutput {
    type Error = anyhow::Error;
    fn try_from(value: RenderOutput) -> Result<Self, Self::Error> {
        Ok(Self {
            render_id: value.render_id,
            diffuse: decode_image_from_base64(&value.diffuse)?,
            depth: value
                .depth
                .map(PrismRenderDepthOutput::try_from)
                .transpose()?,
        })
    }
}

#[derive(Component, ExtractComponent, Clone, Copy)]
struct PrismMainCamera;

#[derive(Component, ExtractComponent, Clone, Copy)]
struct PrismMaskCamera;

#[derive(Clone, Default)]
struct PrismCapturePayload {
    render: Vec<u8>,
    depth: Vec<u8>,
    mask: Vec<u8>,
    camera_position: Vec3,
    camera_rotation: Quat,
    camera_projection: Mat4,
    near_plane: f32,
    far_plane: f32,
}

#[derive(Resource, Clone, ExtractResource)]
struct PrismPostProcessRender {
    capture_rx: crossbeam_channel::Receiver<(Vec3, Quat, Mat4, f32, f32)>,
    result_tx: crossbeam_channel::Sender<PrismCapturePayload>,
}

#[derive(Resource, Clone)]
struct PrismPostProcessGame {
    capture_tx: crossbeam_channel::Sender<(Vec3, Quat, Mat4, f32, f32)>,
    result_rx: crossbeam_channel::Receiver<PrismCapturePayload>,
}

fn startup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.insert_resource(PrismPreviewMesh(meshes.add(Cuboid::new(1.0, 1.0, 1.0))));
    commands.insert_resource(PrismPreviewMaterial(
        materials.add(Color::srgb_u8(124, 144, 255)),
    ));
}

fn spawn_prism(
    render_size: Res<PrismRenderSize>,
    mut egui_user_textures: ResMut<EguiUserTextures>,
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
) {
    // Initial state
    let (result_tx, result_rx) = crossbeam_channel::unbounded();
    let (capture_tx, capture_rx) = crossbeam_channel::unbounded();
    commands.insert_resource(PrismPostProcessRender {
        result_tx,
        capture_rx,
    });
    commands.insert_resource(PrismPostProcessGame {
        result_rx,
        capture_tx,
    });
    commands.insert_resource(PrismState::default());

    let size = Extent3d {
        width: render_size.x,
        height: render_size.y,
        ..default()
    };

    // arbitrary, will be updated by the camera system
    let transform =
        Transform::from_translation(Vec3::new(0.0, 0.0, 15.0)).looking_at(Vec3::default(), Vec3::Y);

    let mut main_image = Image {
        texture_descriptor: TextureDescriptor {
            label: None,
            size,
            dimension: TextureDimension::D2,
            format: TextureFormat::Bgra8UnormSrgb,
            mip_level_count: 1,
            sample_count: 1,
            usage: TextureUsages::TEXTURE_BINDING
                | TextureUsages::COPY_DST
                | TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        },
        ..default()
    };
    main_image.resize(size);
    let image_handle = images.add(main_image);
    egui_user_textures.add_image(image_handle.clone());
    commands.insert_resource(PrismMainImage(image_handle.clone()));
    commands.spawn((
        Camera3dBundle {
            camera: Camera {
                order: 2,
                target: RenderTarget::Image(image_handle),
                clear_color: ClearColorConfig::Custom(Color::srgba(1.0, 1.0, 1.0, 1.0)),
                ..default()
            },
            transform,
            ..default()
        },
        RenderLayers::default(),
        DepthPrepass,
        PrismMainCamera,
    ));

    let mut mask_image = Image {
        texture_descriptor: TextureDescriptor {
            label: None,
            size,
            dimension: TextureDimension::D2,
            format: TextureFormat::Bgra8UnormSrgb,
            mip_level_count: 1,
            sample_count: 1,
            usage: TextureUsages::TEXTURE_BINDING
                | TextureUsages::COPY_DST
                | TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        },
        ..default()
    };
    mask_image.resize(size);
    let mask_image_handle = images.add(mask_image);
    egui_user_textures.add_image(mask_image_handle.clone());
    commands.insert_resource(PrismMaskImage(mask_image_handle.clone()));
    commands.spawn((
        Camera3dBundle {
            camera: Camera {
                order: 3,
                target: RenderTarget::Image(mask_image_handle),
                clear_color: ClearColorConfig::Custom(Color::BLACK),
                ..default()
            },
            transform,
            ..default()
        },
        RenderLayers::layer(rendering::MASK_CAMERA_ONLY_LAYER),
        PrismMaskCamera,
    ));
}

fn activate_camera(
    mut prism_camera: Query<&mut Camera, Or<(With<PrismMainCamera>, With<PrismMaskCamera>)>>,
    active_tool: Res<ActiveTool>,
) {
    for mut camera in prism_camera.iter_mut() {
        camera.is_active = active_tool.is(Tool::Prism);
    }
}

fn sync_game_camera_and_prism_camera(
    game_camera: Query<
        (
            &Projection,
            &Transform,
            &Tonemapping,
            &DebandDither,
            &ColorGrading,
            &Exposure,
        ),
        With<camera::MainCamera>,
    >,
    mut prism_cameras: Query<
        (
            &mut Projection,
            &mut Transform,
            &mut Tonemapping,
            &mut DebandDither,
            &mut ColorGrading,
            &mut Exposure,
        ),
        (
            Or<(With<PrismMainCamera>, With<PrismMaskCamera>)>,
            Without<camera::MainCamera>,
        ),
    >,
) {
    let Ok((
        game_projection,
        game_transform,
        game_tonemapping,
        game_deband_dither,
        game_color_grading,
        game_exposure,
    )) = game_camera.get_single()
    else {
        return;
    };

    for (
        mut prism_projection,
        mut prism_transform,
        mut prism_tonemapping,
        mut prism_deband_dither,
        mut prism_color_grading,
        mut prism_exposure,
    ) in prism_cameras.iter_mut()
    {
        if let (Projection::Perspective(prism), Projection::Perspective(game)) =
            (&mut *prism_projection, game_projection)
        {
            prism.fov = game.fov;
            prism.near = game.near;
            prism.far = game.far;
        }
        *prism_transform = *game_transform;
        *prism_tonemapping = *game_tonemapping;
        *prism_deband_dither = *game_deband_dither;
        *prism_exposure = *game_exposure;
        *prism_color_grading = game_color_grading.clone();
    }
}

fn on_use(
    prism_game: Res<PrismPostProcessGame>,
    prism_state: Res<PrismState>,
    prism_entity: Query<(&GlobalTransform, &Projection), With<PrismMainCamera>>,
) {
    let Ok((global_transform, projection)) = prism_entity.get_single() else {
        return;
    };
    if !prism_state.is_waiting_for_capture() {
        return;
    }
    let camera_projection = projection.get_clip_from_view();
    let (near_plane, far_plane) = if let Projection::Perspective(p) = projection {
        (p.near, p.far)
    } else {
        panic!("Prism camera is not a perspective projection");
    };

    let (_, rotation, translation) = global_transform.to_scale_rotation_translation();
    prism_game
        .capture_tx
        .send((
            translation,
            rotation,
            camera_projection,
            near_plane,
            far_plane,
        ))
        .unwrap();
    info!("Sending capture request");
}

fn update_state_from_events(
    render_size: Res<PrismRenderSize>,
    paint: Res<PrismPaintSettings>,

    mut contexts: bevy_egui::EguiContexts,
    prism_game: Res<PrismPostProcessGame>,
    mut prism_state: ResMut<PrismState>,
    mut toasts: ResMut<Toasts>,
    mut active_tool: ResMut<ActiveTool>,

    mut render_complete: EventReader<PrismRenderOutput>,
    mut projection_completes: EventReader<ProjectionComplete>,
    mut error: EventReader<PrismError>,

    preview_mesh: Res<PrismPreviewMesh>,
    preview_material: Res<PrismPreviewMaterial>,
    mut commands: Commands,
) {
    let ctx = contexts.ctx_mut();

    for error in error.read() {
        *prism_state = PrismState::Error {
            message: error.message.clone(),
        };
    }

    let is_active = active_tool.is(Tool::Prism);

    match &*prism_state {
        PrismState::WaitingForCapture { .. } => {
            for payload in prism_game.result_rx.try_iter() {
                info!(
                    "Received data: render {}b depth {}b mask {}b",
                    payload.render.len(),
                    payload.depth.len(),
                    payload.mask.len()
                );

                let render_image = image::DynamicImage::from(
                    image::RgbaImage::from_vec(render_size.x, render_size.y, payload.render)
                        .expect("Failed to create image from render data"),
                );
                let render_egui =
                    convert_image_to_egui_texture_handle(ctx, "prism_render", &render_image);

                let depth_data = payload
                    .depth
                    .chunks_exact(4)
                    .map(|chunk| f32::from_ne_bytes(chunk.try_into().unwrap()))
                    .collect::<Vec<_>>();
                let depth = convert_depth_buffer_to_normalized_rgba8(
                    &depth_data,
                    render_size.x,
                    render_size.y,
                );
                let depth_egui =
                    convert_image_to_egui_texture_handle(ctx, "prism_depth", &depth.image);

                let mask_image = image::DynamicImage::from(
                    image::RgbaImage::from_vec(render_size.x, render_size.y, payload.mask)
                        .expect("Failed to create image from mask data"),
                );

                let mask_texture = ctx.load_texture(
                    "prism_capture_mask",
                    egui::ColorImage::new(
                        [render_size.x as usize, render_size.y as usize],
                        egui::Color32::TRANSPARENT,
                    ),
                    egui::TextureOptions::default(),
                );

                *prism_state = PrismState::Capture(PrismStateCapture {
                    camera_position: payload.camera_position,
                    camera_rotation: payload.camera_rotation,
                    camera_projection: payload.camera_projection,
                    near_plane: payload.near_plane,
                    far_plane: payload.far_plane,
                    render: render_image,
                    render_egui,
                    depth,
                    depth_egui,
                    depth_data,
                    mask: mask_image
                        .pixels()
                        .map(|(_, _, p)| p.0[0..3].iter().map(|v| *v as u32).sum::<u32>() > 0)
                        .collect::<_>(),
                    mask_texture,
                    prompt: paint.prompt.clone(),
                    show_depth: false,
                });

                if !is_active {
                    toasts
                        .info("Prism captured. Ready to reimagine.")
                        .set_duration(Some(Duration::from_secs(2)));
                }
            }
        }
        PrismState::Capture(_) => {}
        PrismState::Rendering { input, mask, .. } => {
            for render in render_complete.read() {
                match handle_render_complete_event(
                    ctx,
                    input,
                    render,
                    mask,
                    &preview_mesh,
                    &preview_material,
                    &mut commands,
                ) {
                    Ok(Some(state)) => {
                        *prism_state = state;

                        if !is_active {
                            toasts
                                .info("Prism reimagination complete. Ready to project.")
                                .set_duration(Some(Duration::from_secs(2)));
                        }

                        // We intentionally ignore all future events for this state;
                        // it's not clear what it means to receive events once you're already transitioned out,
                        // and it causes borrow checker issues
                        return;
                    }
                    Ok(None) => {}
                    Err(err) => {
                        *prism_state = PrismState::Error {
                            message: err.to_string(),
                        };
                        return;
                    }
                }
            }
        }
        PrismState::Rendered(_) => {}
        PrismState::Projecting { request_id, .. } => {
            let request_id = *request_id;
            for projection_complete in projection_completes.read() {
                if projection_complete.request_id != request_id {
                    continue;
                }

                *prism_state = PrismState::default();

                if !is_active {
                    toasts
                        .info("Prism successfully projected.")
                        .set_duration(Some(Duration::from_secs(3)));
                } else {
                    active_tool.disable_if_active(Tool::Prism);
                }
            }
        }
        PrismState::Error { .. } => {}
    }
}

fn handle_render_complete_event(
    ctx: &egui::Context,
    input: &RenderInput,
    render: &PrismRenderOutput,
    mask: &[bool],
    preview_mesh: &PrismPreviewMesh,
    preview_material: &PrismPreviewMaterial,
    commands: &mut Commands,
) -> anyhow::Result<Option<PrismState>> {
    if input.render_id != render.render_id {
        warn!(
            "Received render {}, but waiting for render {}; ignoring",
            input.render_id, render.render_id
        );
        return Ok(None);
    }

    let diffuse = &render.diffuse;

    let render_egui = convert_image_to_egui_texture_handle(ctx, "prism_ai_render", diffuse);

    let depth_egui = render
        .depth
        .as_ref()
        .map(|d| convert_image_to_egui_texture_handle(ctx, "prism_ai_depth", &d.relative_depth));

    let (size_x, size_y) = (diffuse.width() as usize, diffuse.height() as usize);
    let mask_texture = ctx.load_texture(
        "prism_capture_mask",
        egui::ColorImage::new([size_x, size_y], egui::Color32::TRANSPARENT),
        egui::TextureOptions::default(),
    );

    let preview_entity = commands
        .spawn((
            PbrBundle {
                mesh: preview_mesh.0.clone(),
                material: preview_material.0.clone(),
                transform: Transform {
                    translation: Vec3::from_array(input.camera_position),
                    rotation: Quat::from_array(input.camera_rotation),
                    scale: Vec3::ONE,
                },
                ..default()
            },
            bevy_mod_picking::PickableBundle::default(),
        ))
        .id();

    Ok(Some(PrismState::Rendered(PrismStateRendered {
        input: input.clone(),
        render: diffuse.clone(),
        render_egui,
        depth: render.depth.clone(),
        depth_egui,
        mask: mask.to_vec(),
        mask_texture,
        show_depth: false,
        preview_entity,
        use_estimation: true,
        scale: 1.0,
    })))
}

fn ui(
    mut contexts: bevy_egui::EguiContexts,
    mut reqwest: BevyReqwest,
    mut prism_state: ResMut<PrismState>,
    main_image: Option<Res<PrismMainImage>>,
    mask_image: Option<Res<PrismMaskImage>>,
    mut paint: ResMut<PrismPaintSettings>,
    global_transform_query: Query<&GlobalTransform>,

    mut export_image_requests: EventWriter<ExportImage>,
    mut export_depth_requests: EventWriter<ExportDepth>,
    mut export_rendered_image_requests: EventWriter<ExportRenderedImage>,
    mut export_rendered_depth_requests: EventWriter<ExportRenderedDepth>,
    mut prism_render_output_requests: EventWriter<PrismRenderOutput>,

    projection_requests: EventWriter<ProjectionRequest>,
    http_endpoints: Res<HttpEndpoints>,

    mut commands: Commands,
) {
    const REIMAGINE_ICON: &str = egui_phosphor::regular::MAGIC_WAND;
    const RETAKE_ICON: &str = egui_phosphor::regular::CAMERA;
    const PROJECT_ICON: &str = egui_phosphor::regular::SUN;
    const EXPORT_IMAGE_ICON: &str = egui_phosphor::regular::EXPORT;
    const EXPORT_DEPTH_ICON: &str = egui_phosphor::regular::ARROW_SQUARE_OUT;

    let Some(preview_image) = main_image else {
        warn!("Prism UI was requested, but prism state has not been initialised yet");
        return;
    };

    let mask_image = mask_image.unwrap();
    let preview_texture_id = contexts.image_id(&preview_image.0).unwrap();
    let mask_texture_id = contexts.image_id(&mask_image.0).unwrap();

    let ctx = contexts.ctx_mut();

    let screen_size = ctx.screen_rect().size();
    let total_size = screen_size.min_elem();
    let image_size_pct = egui::Vec2::splat(0.75);
    let side_panel_width_pct = 0.15;
    let window_size_pct =
        image_size_pct + egui::vec2(side_panel_width_pct, 0.0) + egui::vec2(0.10, 0.05);

    let image_size = total_size * image_size_pct;
    let side_panel_width = total_size * side_panel_width_pct;
    let window_size = total_size * window_size_pct;

    pub fn side_panel<R>(
        ui: &mut egui::Ui,
        id: &'static str,
        side_panel_width: f32,
        add_contents: impl FnOnce(&mut egui::Ui) -> R,
    ) -> egui::InnerResponse<R> {
        egui::SidePanel::right(id)
            .min_width(side_panel_width)
            .resizable(false)
            .show_inside(ui, |ui| {
                ui.with_layout(
                    egui::Layout::top_down_justified(egui::Align::LEFT),
                    add_contents,
                )
                .inner
            })
    }

    pub fn central_panel<R>(
        ui: &mut egui::Ui,
        add_contents: impl FnOnce(&mut egui::Ui) -> R,
    ) -> egui::InnerResponse<R> {
        egui::CentralPanel::default().show_inside(ui, add_contents)
    }

    fn centered_window(
        ctx: &mut egui::Context,
        title: &str,
        id: &'static str,
        window_size: Option<egui::Vec2>,
        f: impl FnMut(&mut egui::Ui),
    ) {
        let window = egui::Window::new(title)
            .id(id.into())
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .collapsible(false)
            .resizable(true);

        if let Some(window_size) = window_size {
            window.min_size(window_size)
        } else {
            window
        }
        .show(ctx, f);
    }

    fn main_window(ctx: &mut egui::Context, window_size: egui::Vec2, f: impl FnMut(&mut egui::Ui)) {
        centered_window(ctx, "Prism", "prism_main", Some(window_size), f);
    }

    match &*prism_state {
        PrismState::WaitingForCapture { show_mask } => {
            let mut new_show_mask = *show_mask;
            let mut load_json_requested = false;

            main_window(ctx, window_size, |ui| {
                side_panel(ui, "prism_wfc_right", side_panel_width, |ui| {
                    ui.label("Click to capture.");
                    ui.checkbox(&mut new_show_mask, "Show mask");
                    if ui.button("[DEBUG] Load JSON").clicked() {
                        load_json_requested = true;
                    }
                });

                central_panel(ui, |ui| {
                    ui.image(egui::load::SizedTexture::new(
                        if new_show_mask {
                            mask_texture_id
                        } else {
                            preview_texture_id
                        },
                        image_size,
                    ));
                });
            });

            if new_show_mask != *show_mask {
                *prism_state = PrismState::WaitingForCapture {
                    show_mask: new_show_mask,
                };
            }

            if load_json_requested {
                let json = std::fs::read_to_string("prism_render.json").unwrap();
                let render: SerializablePrismRenderOutput = serde_json::from_str(&json).unwrap();
                *prism_state = PrismState::try_from(&render).unwrap();
                prism_render_output_requests.send(PrismRenderOutput::try_from(render).unwrap());
            }
        }
        PrismState::Capture(_) => {
            let PrismState::Capture(capture) = &mut *prism_state else {
                return;
            };
            let mut reimagine_requested = false;
            let mut retake_requested = false;

            main_window(ctx, window_size, |ui| {
                side_panel(ui, "prism_capture_right", side_panel_width, |ui| {
                    ui.horizontal(|ui| {
                        if ui
                            .add(egui::Button::new(format!("{RETAKE_ICON} Retake")))
                            .clicked()
                        {
                            retake_requested = true;
                        }
                        let reimagine_enabled = capture.mask.iter().any(|&b| b);
                        if ui
                            .add_enabled(
                                reimagine_enabled,
                                egui::Button::new(format!("{REIMAGINE_ICON} Reimagine")),
                            )
                            .clicked()
                        {
                            reimagine_requested = true;
                        }
                    });

                    ui.horizontal(|ui| {
                        if ui
                            .add(egui::Button::new(format!(
                                "{EXPORT_IMAGE_ICON} Export image"
                            )))
                            .clicked()
                        {
                            export_image_requests.send(ExportImage);
                        }
                        if ui
                            .add(egui::Button::new(format!(
                                "{EXPORT_DEPTH_ICON} Export depth"
                            )))
                            .clicked()
                        {
                            export_depth_requests.send(ExportDepth);
                        }
                    });

                    ui.label("Prompt");
                    let textbox_response = ui.add(
                        egui::TextEdit::multiline(&mut capture.prompt)
                            .vertical_align(egui::Align::Center),
                    );
                    if textbox_response.lost_focus()
                        && ui.input(|i| i.key_pressed(egui::Key::Enter))
                    {
                        reimagine_requested = true;
                    }

                    ui.label("Seed");
                    ui.add(egui::DragValue::new(&mut paint.seed));
                    ui.label("Repaint%");
                    ui.add(
                        egui::DragValue::new(&mut paint.repaint_amount)
                            .range(0.0..=1.0)
                            .fixed_decimals(2)
                            .speed(0.01),
                    );
                    ui.label("Steps");
                    ui.add(
                        egui::DragValue::new(&mut paint.sampler_steps)
                            .speed(1.0)
                            .range(2u8..=25),
                    );
                    ui.label("Depth Strength");
                    ui.add(
                        egui::DragValue::new(&mut paint.depth_controlnet_strength)
                            .range(0.5..=1.5)
                            .fixed_decimals(2)
                            .speed(0.01),
                    );
                    ui.label("Show Depth");
                    ui.checkbox(&mut capture.show_depth, "");
                });

                let show_depth = capture.show_depth;
                central_panel(ui, |ui| {
                    let texture = egui::load::SizedTexture::from_handle(if show_depth {
                        &capture.depth_egui
                    } else {
                        &capture.render_egui
                    });
                    ui.add(ImageMask::new(
                        texture,
                        image_size,
                        &mut capture.mask,
                        capture.mask_texture.clone(),
                        &capture.render,
                    ));
                });
            });

            if reimagine_requested {
                let start_time = web_time::Instant::now();
                paint.prompt = capture.prompt.clone();
                let input = match reimagine(&mut reqwest, &http_endpoints, &paint, capture) {
                    Ok(id) => id,
                    Err(err) => {
                        *prism_state = PrismState::Error {
                            message: err.to_string(),
                        };
                        return;
                    }
                };
                *prism_state = PrismState::Rendering {
                    render_egui: Some(capture.render_egui.clone()),
                    start_time,
                    input,
                    mask: capture.mask.clone(),
                };
            } else if retake_requested {
                *prism_state = PrismState::default();
            }
        }
        PrismState::Rendering {
            render_egui,
            start_time,
            input: _,
            mask: _,
        } => {
            let mut retake_requested = false;
            let elapsed_time = start_time.elapsed().as_secs_f32();
            main_window(ctx, window_size, |ui| {
                side_panel(ui, "prism_rendering_right", side_panel_width, |ui| {
                    if ui
                        .add(egui::Button::new(format!(
                            "{RETAKE_ICON} Cancel and retake"
                        )))
                        .clicked()
                    {
                        retake_requested = true;
                    }
                    ui.add(egui::Label::new(format!(
                        "Reimagining... ({:.1}s)",
                        elapsed_time
                    )));
                });

                central_panel(ui, |ui| {
                    if let Some(render_egui) = render_egui {
                        ui.add(
                            egui::Image::new(egui::load::SizedTexture::from_handle(render_egui))
                                .max_size(image_size)
                                .tint(egui::ecolor::Hsva::new(
                                    (0.5 * elapsed_time) % 1.0,
                                    0.5,
                                    0.5,
                                    1.0,
                                )),
                        );
                    }
                });
            });

            if retake_requested {
                *prism_state = PrismState::default();
            }
        }
        PrismState::Rendered(rendered) => {
            let mut project_requested = false;
            let mut retake_requested = false;

            let mut updated_mask = None;
            let mut update_show_depth = None;
            let mut update_use_estimation = None;
            let mut updated_scale = None;
            let mut export_json_requested = false;

            egui::Window::new("Prism")
                .id("prism_rendered".into())
                .pivot(egui::Align2::CENTER_BOTTOM)
                .default_pos(egui::pos2(screen_size.x / 2.0, screen_size.y))
                .collapsible(false)
                .resizable(true)
                .show(ctx, |ui| {
                    let image = if let Some(depth_handle) =
                        rendered.depth_egui.as_ref().filter(|_| rendered.show_depth)
                    {
                        depth_handle
                    } else {
                        &rendered.render_egui
                    };
                    let mut mask = rendered.mask.clone();
                    ui.add(ImageMask::new(
                        egui::load::SizedTexture::from_handle(image),
                        image_size * 0.6,
                        &mut mask,
                        rendered.mask_texture.clone(),
                        &rendered.render,
                    ));
                    if mask != rendered.mask {
                        updated_mask = Some(mask);
                    }

                    ui.horizontal(|ui| {
                        let project_enabled = rendered.mask.iter().any(|&b| b);
                        if ui
                            .add_enabled(
                                project_enabled,
                                egui::Button::new(format!("{PROJECT_ICON} Project")),
                            )
                            .clicked()
                        {
                            project_requested = true;
                        }

                        if ui
                            .add(egui::Button::new(format!("{RETAKE_ICON} Retake")))
                            .clicked()
                        {
                            retake_requested = true;
                        }

                        ui.separator();

                        if ui
                            .add(egui::Button::new(format!(
                                "{EXPORT_IMAGE_ICON} Export image"
                            )))
                            .clicked()
                        {
                            export_rendered_image_requests.send(ExportRenderedImage);
                        }
                        if ui
                            .add(egui::Button::new(format!(
                                "{EXPORT_DEPTH_ICON} Export depth"
                            )))
                            .clicked()
                        {
                            export_rendered_depth_requests.send(ExportRenderedDepth);
                        }
                    });
                    ui.horizontal(|ui| {
                        if ui.button("[DEBUG] Export JSON").clicked() {
                            export_json_requested = true;
                        }

                        if rendered.depth_egui.is_some() {
                            ui.label("Show Depth");
                            let mut show_depth = rendered.show_depth;
                            ui.checkbox(&mut show_depth, "");
                            if show_depth != rendered.show_depth {
                                update_show_depth = Some(show_depth);
                            }
                        }

                        if rendered.depth.is_some() {
                            ui.label("Use Estimation");
                            let mut use_estimation = rendered.use_estimation;
                            ui.checkbox(&mut use_estimation, "");
                            if use_estimation != rendered.use_estimation {
                                update_use_estimation = Some(use_estimation);
                            }
                        }
                    });

                    let mut scale = rendered.scale;
                    ui.add(
                        egui::Slider::new(&mut scale, 0.0001..=1.0)
                            .logarithmic(true)
                            .text("Scale"),
                    );
                    if scale != rendered.scale {
                        updated_scale = Some(scale);
                    }
                });

            if export_json_requested {
                std::fs::write(
                    "prism_render.json",
                    serde_json::to_string(
                        &SerializablePrismRenderOutput::try_from(rendered).unwrap(),
                    )
                    .unwrap(),
                )
                .unwrap();
            }

            if project_requested {
                let request_id = match project(
                    projection_requests,
                    rendered,
                    global_transform_query.get(rendered.preview_entity).unwrap(),
                ) {
                    Ok(id) => id,
                    Err(err) => {
                        *prism_state = PrismState::Error {
                            message: err.to_string(),
                        };
                        return;
                    }
                };

                commands.entity(rendered.preview_entity).despawn_recursive();

                *prism_state = PrismState::Projecting {
                    start_time: web_time::Instant::now(),
                    request_id,
                };
            } else if retake_requested {
                commands.entity(rendered.preview_entity).despawn_recursive();
                *prism_state = PrismState::default();
            }
            // These are separate to avoid mutably borrowing the state unless necessary
            if let Some(mask) = updated_mask {
                if let PrismState::Rendered(rendered) = &mut *prism_state {
                    rendered.mask = mask;
                }
            }
            if let Some(show_depth) = update_show_depth {
                if let PrismState::Rendered(rendered) = &mut *prism_state {
                    rendered.show_depth = show_depth;
                }
            }
            if let Some(use_estimation) = update_use_estimation {
                if let PrismState::Rendered(rendered) = &mut *prism_state {
                    rendered.use_estimation = use_estimation;
                }
            }
            if let Some(scale) = updated_scale {
                if let PrismState::Rendered(rendered) = &mut *prism_state {
                    rendered.scale = scale;
                }
            }
        }
        PrismState::Projecting { start_time, .. } => {
            centered_window(ctx, "Prism", "prism_projecting", None, |ui| {
                let elapsed_time = start_time.elapsed().as_secs_f32();
                ui.label(format!("Projecting... ({:.1}s)", elapsed_time));
            });
        }
        PrismState::Error { message } => {
            let mut close = false;
            centered_window(ctx, "Prism", "prism_error", None, |ui| {
                egui::ScrollArea::both().show(ui, |ui| {
                    ui.label(format!("Error: {message}"));
                });
                if ui.button("Ok").clicked() {
                    close = true;
                }
            });
            if close {
                *prism_state = PrismState::default();
            }
        }
    }
}

fn reimagine(
    client: &mut BevyReqwest,
    http_endpoints: &HttpEndpoints,
    paint: &PrismPaintSettings,
    capture: &PrismStateCapture,
) -> Result<RenderInput, image::ImageError> {
    let render_id: u64 = rand::random();
    let depth_image = &capture.depth.image;
    let input = RenderInput {
        render_id,
        camera_position: capture.camera_position.to_array(),
        camera_rotation: capture.camera_rotation.to_array(),
        camera_projection: capture.camera_projection.to_cols_array(),
        prompt: capture.prompt.clone(),
        seed: paint.seed,
        repaint_amount: paint.repaint_amount,
        sampler_steps: paint.sampler_steps,
        depth_controlnet_strength: paint.depth_controlnet_strength,
        depth_raw: (
            depth_image.width(),
            depth_image.height(),
            capture.depth_data.clone(),
        ),
        depth: encode_image_to_base64(depth_image)?,
        depth_min: capture.depth.min,
        depth_max: capture.depth.max,
        near_plane: capture.near_plane,
        far_plane: capture.far_plane,
        base_render: encode_image_to_base64(&capture.render)?,
        mask: encode_image_to_base64(
            &mask_to_image(
                &capture.mask,
                depth_image.width() as usize,
                depth_image.height() as usize,
            )
            .into(),
        )?,
    };

    client
        .send(
            client
                .post(&http_endpoints.render_url)
                .json(&input)
                .build()
                .unwrap(),
        )
        .on_response(
            |trigger: Trigger<ReqwestResponseEvent>,
             mut output: EventWriter<PrismRenderOutput>,
             mut error: EventWriter<PrismError>| {
                let response = trigger.event();
                let result =
                    response
                        .deserialize_json::<RenderOutputResult>()
                        .and_then(|r| match r {
                            RenderOutputResult::Ok(render_output) => {
                                Ok(PrismRenderOutput::try_from(render_output)?)
                            }
                            RenderOutputResult::Err(err) => anyhow::bail!("{err}"),
                        });

                match result {
                    Ok(r) => {
                        output.send(r);
                    }
                    Err(e) => {
                        error.send(PrismError {
                            message: format!("{e:?}"),
                        });
                    }
                }
            },
        )
        .on_error(
            |trigger: Trigger<ReqwestErrorEvent>, mut errors: EventWriter<PrismError>| {
                errors.send(PrismError {
                    message: format!("{:?}", trigger.event().0),
                });
            },
        );

    Ok(input)
}

fn project(
    mut projection_requests: EventWriter<ProjectionRequest>,
    rendered: &PrismStateRendered,
    global_transform: &GlobalTransform,
) -> Result<u64, image::ImageError> {
    let request = ProjectionRequest::from_rendered(rendered, global_transform);
    let request_id = request.request_id;
    projection_requests.send(request);
    Ok(request_id)
}

#[derive(Event)]
pub struct ExportImage;
#[derive(Clone, Copy)]
pub struct ExportImageHandler;
impl WriteHandler for ExportImageHandler {
    type Event = ExportImage;

    fn filename(&self, _world: &World) -> Option<String> {
        Some("prism_render.png".into())
    }
    fn on_save_without_filename(&self, world: &mut World) {
        world.resource_mut::<Toasts>().info("Export cancelled.");
    }
    fn write(&self, world: &mut World) -> Vec<u8> {
        if let PrismState::Capture(capture) = world.resource::<PrismState>() {
            encode_image_to_png(&capture.render).unwrap()
        } else {
            warn!("Export requested, but no captured image is available");
            vec![]
        }
    }
    fn on_write_complete(&self, world: &mut World, result: std::io::Result<()>) {
        if let Err(err) = result {
            world
                .resource_mut::<Toasts>()
                .error(format!("Export failed: {:?}", err));
        } else {
            world.resource_mut::<Toasts>().info("Export complete.");
        }
    }
}

#[derive(Event)]
pub struct ExportDepth;
#[derive(Clone, Copy)]
pub struct ExportDepthHandler;
impl WriteHandler for ExportDepthHandler {
    type Event = ExportDepth;

    fn filename(&self, _world: &World) -> Option<String> {
        Some("prism_depth.png".into())
    }
    fn on_save_without_filename(&self, world: &mut World) {
        world.resource_mut::<Toasts>().info("Export cancelled.");
    }
    fn write(&self, world: &mut World) -> Vec<u8> {
        if let PrismState::Capture(capture) = world.resource::<PrismState>() {
            encode_image_to_png(&capture.depth.image).unwrap()
        } else {
            warn!("Export requested, but no captured depth is available");
            vec![]
        }
    }
    fn on_write_complete(&self, world: &mut World, result: std::io::Result<()>) {
        if let Err(err) = result {
            world
                .resource_mut::<Toasts>()
                .error(format!("Export failed: {:?}", err));
        } else {
            world.resource_mut::<Toasts>().info("Export complete.");
        }
    }
}

#[derive(Event)]
pub struct ExportRenderedImage;
#[derive(Clone, Copy)]
pub struct ExportRenderedImageHandler;
impl WriteHandler for ExportRenderedImageHandler {
    type Event = ExportRenderedImage;

    fn filename(&self, _world: &World) -> Option<String> {
        Some("prism_ai_render.png".into())
    }
    fn on_save_without_filename(&self, world: &mut World) {
        world.resource_mut::<Toasts>().info("Export cancelled.");
    }
    fn write(&self, world: &mut World) -> Vec<u8> {
        if let PrismState::Rendered(rendered) = world.resource::<PrismState>() {
            encode_image_to_png(&rendered.render).unwrap()
        } else {
            warn!("Export requested, but no rendered image is available");
            vec![]
        }
    }
    fn on_write_complete(&self, world: &mut World, result: std::io::Result<()>) {
        if let Err(err) = result {
            world
                .resource_mut::<Toasts>()
                .error(format!("Export failed: {:?}", err));
        } else {
            world.resource_mut::<Toasts>().info("Export complete.");
        }
    }
}

#[derive(Event)]
pub struct ExportRenderedDepth;
#[derive(Clone, Copy)]
pub struct ExportRenderedDepthHandler;
impl WriteHandler for ExportRenderedDepthHandler {
    type Event = ExportRenderedDepth;

    fn filename(&self, _world: &World) -> Option<String> {
        Some("prism_ai_depth.png".into())
    }
    fn on_save_without_filename(&self, world: &mut World) {
        world.resource_mut::<Toasts>().info("Export cancelled.");
    }
    fn write(&self, world: &mut World) -> Vec<u8> {
        if let PrismState::Rendered(PrismStateRendered {
            depth: Some(depth), ..
        }) = world.resource::<PrismState>()
        {
            encode_image_to_png(&depth.relative_depth).unwrap()
        } else {
            warn!("Export requested, but no rendered depth is available");
            vec![]
        }
    }
    fn on_write_complete(&self, world: &mut World, result: std::io::Result<()>) {
        if let Err(err) = result {
            world
                .resource_mut::<Toasts>()
                .error(format!("Export failed: {:?}", err));
        } else {
            world.resource_mut::<Toasts>().info("Export complete.");
        }
    }
}

pub fn ui_top_left_panel(ui: &mut egui::Ui, _world: &World) {
    ui.label("Use your prism to reimagine reality.");
}

pub struct NormalizedOutput {
    pub image: image::DynamicImage,
    pub min: f32,
    pub max: f32,
}
pub fn convert_depth_buffer_to_normalized_rgba8(
    depth_data: &[f32],
    width: u32,
    height: u32,
) -> NormalizedOutput {
    let depth: image::DynamicImage = image::ImageBuffer::<image::Luma<u16>, _>::from_raw(
        width,
        height,
        depth_data
            .iter()
            .map(|v| (*v * 65535.0) as u16)
            .collect::<Vec<_>>(),
    )
    .unwrap()
    .into();

    let mut processed_depth = depth
        .resize(width, height, image::imageops::FilterType::Lanczos3)
        .into_luma16();

    let min = processed_depth.pixels().map(|p| p[0]).min().unwrap();
    let max = processed_depth.pixels().map(|p| p[0]).max().unwrap();
    for pixel in processed_depth.pixels_mut() {
        pixel[0] =
            65535 - (((pixel[0] as f32 - min as f32) * 65535.0 / (max as f32 - min as f32)) as u16);
    }

    NormalizedOutput {
        image: image::DynamicImage::from(processed_depth).to_rgba8().into(),
        min: min as f32 / 65535.0,
        max: max as f32 / 65535.0,
    }
}

fn convert_image_to_egui_texture_handle(
    ctx: &egui::Context,
    id: &str,
    image: &image::DynamicImage,
) -> egui::TextureHandle {
    ctx.load_texture(
        id,
        egui::ColorImage::from_rgba_unmultiplied(
            [image.width() as usize, image.height() as usize],
            image.to_rgba8().as_bytes(),
        ),
        egui::TextureOptions::LINEAR,
    )
}
