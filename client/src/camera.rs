use bevy::{
    camera::{visibility::RenderLayers, ClearColorConfig},
    input::mouse::{MouseMotion, MouseWheel},
    pbr::Atmosphere,
    picking::mesh_picking::ray_cast::MeshRayCast,
    prelude::*,
};
use bevy_egui::{egui, EguiGlobalSettings, PrimaryEguiContext};

use crate::{
    raycast::{raycast, RaycastIgnore},
    rendering::{
        draw_gizmo_cross, MainCameraGizmos, ALL_NON_MASK_CAMERA_LAYER, MAIN_CAMERA_ONLY_LAYER,
    },
    ui::{is_cursor_invisible, normalize_scroll_wheel},
    voxel::{ChunkData, Chunks, VoxelSizeMeters, VoxelsPerMeter},
};

#[derive(Component)]
pub struct MainCamera;

#[derive(Component)]
pub struct SecondaryCamera;

#[derive(Component, PartialEq, Eq, Clone, Copy)]
pub enum CameraType {
    Free,
    Orbit,
}

impl std::fmt::Display for CameraType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CameraType::Free => write!(f, "Free"),
            CameraType::Orbit => write!(f, "Orbit"),
        }
    }
}

/// Camera controller component that stores camera state
#[derive(Component)]
pub struct CameraController {
    pub position: Vec3,
    pub yaw: f32,     // radians
    pub pitch: f32,   // radians
    pub target: Vec3, // For orbit camera
}

impl CameraController {
    pub fn new_fpv(position: Vec3, looking_at: Vec3) -> Self {
        let direction = (looking_at - position).normalize();
        let yaw = direction.x.atan2(direction.z);
        let pitch = (-direction.y).asin();

        Self {
            position,
            yaw,
            pitch,
            target: looking_at,
        }
    }

    pub fn new_orbit(position: Vec3, target: Vec3) -> Self {
        Self {
            position,
            yaw: 0.0,
            pitch: 0.0,
            target,
        }
    }

    pub fn distance_to_target(&self) -> f32 {
        self.position.distance(self.target)
    }
}

/// System set for camera setup, used for ordering other systems that depend on the egui context.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct CameraSetup;

pub fn plugin(app: &mut App) {
    app.add_systems(Startup, setup.in_set(CameraSetup))
        .add_systems(
        Update,
        (
            swap_camera,
            update_camera.run_if(is_cursor_invisible),
            sync_primary_and_secondary_camera_transforms,
            apply_camera_controller,
            draw_orbit_camera_target.run_if(is_cursor_invisible),
        )
            .chain(),
    );
}

pub fn ui_top_right_panel(
    ui: &mut egui::Ui,
    main_camera: Query<(&CameraType, &CameraController), With<MainCamera>>,
) {
    let Ok((camera_type, controller)) = main_camera.single() else {
        return;
    };
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Camera:").underline());
        ui.label(camera_type.to_string());
        if *camera_type == CameraType::Orbit {
            let distance = controller.distance_to_target();
            ui.label(format!("({distance:.2}m)"));
        }
    });
    ui.label("Press T to swap.");
}

fn setup(mut commands: Commands, mut egui_settings: ResMut<EguiGlobalSettings>) {
    // Disable auto-creation of egui context so we can create a dedicated egui camera
    // that stays active regardless of which 3D camera is being used.
    egui_settings.auto_create_primary_context = false;

    // Camera
    let target = Vec3::new(0., 0.45, 0.);
    let transform = Transform::from_xyz(-0.45, 0.45, -0.45).looking_at(target, Vec3::Y);
    let render_layers = RenderLayers::from_layers(&[
        ALL_NON_MASK_CAMERA_LAYER as usize,
        MAIN_CAMERA_ONLY_LAYER as usize,
    ]);

    // Both cameras use the same order so they're treated identically by the render pipeline.
    // Only one should be active at a time.
    commands.spawn((
        MainCamera,
        CameraType::Free,
        CameraController::new_fpv(transform.translation, target),
        Camera3d::default(),
        Camera {
            order: 0,
            ..default()
        },
        transform,
        #[cfg(feature = "webgpu")]
        Atmosphere::EARTH,
        render_layers.clone(),
        transform_gizmo_bevy::GizmoCamera,
    ));

    commands.spawn((
        SecondaryCamera,
        CameraType::Orbit,
        CameraController::new_orbit(transform.translation, target),
        Camera3d::default(),
        Camera {
            order: 0,
            is_active: false,
            ..default()
        },
        transform,
        #[cfg(feature = "webgpu")]
        Atmosphere::EARTH,
        render_layers,
    ));

    // Dedicated egui camera - always active, renders last, doesn't clear the screen.
    // This ensures egui UI is always visible regardless of which 3D camera is active.
    commands.spawn((
        PrimaryEguiContext,
        Camera2d,
        Camera {
            order: 100,
            clear_color: ClearColorConfig::None,
            ..default()
        },
    ));
}

fn orbit_min_distance(voxel_size_meters: VoxelSizeMeters) -> f32 {
    5.0 * voxel_size_meters.0
}
fn orbit_max_distance(voxel_size_meters: VoxelSizeMeters) -> f32 {
    2_000.0 * voxel_size_meters.0
}

fn swap_camera(
    keys: Res<ButtonInput<KeyCode>>,
    mut world_raycast: MeshRayCast,
    mut commands: Commands,
    mut q_main: Query<
        (Entity, &mut Camera, &GlobalTransform, &CameraController),
        (With<MainCamera>, Without<SecondaryCamera>),
    >,
    mut q_sec: Query<
        (Entity, &mut Camera, &CameraType, &mut CameraController),
        (With<SecondaryCamera>, Without<MainCamera>),
    >,

    chunk_datas: Query<&ChunkData>,
    raycast_ignores: Query<(), With<RaycastIgnore>>,
    parents: Query<&ChildOf>,

    chunks: Res<Chunks>,
    voxels_per_meter: Res<VoxelsPerMeter>,
    voxel_size_meters: Res<VoxelSizeMeters>,
) {
    if !keys.just_pressed(KeyCode::KeyT) {
        return;
    }
    let Ok((e_main, mut cam_main, transform_main, _controller_main)) = q_main.single_mut() else {
        return;
    };
    let Ok((e_sec, mut cam_sec, type_sec, mut controller_sec)) = q_sec.single_mut() else {
        return;
    };
    commands
        .entity(e_main)
        .remove::<MainCamera>()
        .remove::<transform_gizmo_bevy::GizmoCamera>()
        .insert(SecondaryCamera);
    commands
        .entity(e_sec)
        .remove::<SecondaryCamera>()
        .insert(transform_gizmo_bevy::GizmoCamera)
        .insert(MainCamera);
    cam_sec.is_active = true;
    cam_main.is_active = false;

    let voxels_per_meter = *voxels_per_meter;
    if *type_sec == CameraType::Orbit {
        // Handles the three cases:
        // - we hit a voxel: make that our target, but aim for the centre of the ray intersection
        //   if possible
        // - we hit the ground: orbit 200 * VOXEL_SIZE_METERS in front, or hit if closer
        // - we hit nothing: orbit 200 * VOXEL_SIZE_METERS in front
        let origin = transform_main.translation();
        let direction = *transform_main.forward();
        let hit = raycast(
            &mut world_raycast,
            &chunks,
            &chunk_datas,
            &raycast_ignores,
            &parents,
            *voxel_size_meters,
            Ray3d::new(origin, Dir3::new(direction).unwrap()),
            orbit_max_distance(*voxel_size_meters),
            false,
        );

        let fallback_distance = 200.0 * voxel_size_meters.0;
        let fallback_location = transform_main.translation() + direction * fallback_distance;
        let target = if let Some(hit) = hit {
            let coords = hit.entry_coords.to_world(voxels_per_meter);
            if hit.hit_voxel {
                if let Some(exit_coords) = hit.exit_coords {
                    let exit_coords = exit_coords.to_world(voxels_per_meter);
                    (coords + exit_coords) / 2.0
                } else {
                    coords
                }
            } else if transform_main.translation().distance(coords) < fallback_distance {
                coords
            } else {
                fallback_location
            }
        } else {
            fallback_location
        };
        controller_sec.target = target;
    }
}

pub fn update_camera(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    voxel_size_meters: Res<VoxelSizeMeters>,
    mut mouse_motion_events: MessageReader<MouseMotion>,
    mut scroll_events: MessageReader<MouseWheel>,
    mut main_camera_query: Query<(&CameraType, &mut CameraController), With<MainCamera>>,
) {
    let time_delta_seconds: f32 = time.delta_secs();
    let boost_mult = 5.0f32;
    let slow_mult = 0.25f32;
    let sensitivity = Vec2::splat(1.0);
    let scroll_sensitivity = 0.1;

    let mut move_vec = Vec3::ZERO;

    if keys.pressed(KeyCode::KeyW) {
        move_vec.z -= 1.0;
    }
    if keys.pressed(KeyCode::KeyS) {
        move_vec.z += 1.0;
    }
    if keys.pressed(KeyCode::KeyA) {
        move_vec.x -= 1.0;
    }
    if keys.pressed(KeyCode::KeyD) {
        move_vec.x += 1.0;
    }

    if keys.pressed(KeyCode::KeyE) || keys.pressed(KeyCode::Space) {
        move_vec.y += 1.0;
    }
    if keys.pressed(KeyCode::KeyQ) || keys.pressed(KeyCode::ControlLeft) {
        move_vec.y -= 1.0;
    }

    let speed: f32 = if keys.pressed(KeyCode::ShiftLeft) {
        boost_mult
    } else if keys.pressed(KeyCode::AltLeft) {
        slow_mult
    } else {
        1.
    } * voxel_size_meters.0
        * 10.0;

    let mut delta = Vec2::ZERO;
    for event in mouse_motion_events.read() {
        delta += event.delta;
    }
    delta.x *= sensitivity.x;
    delta.y *= sensitivity.y;

    let Ok((camera_type, mut controller)) = main_camera_query.single_mut() else {
        return;
    };

    match camera_type {
        CameraType::Free => {
            // Update yaw and pitch from mouse delta
            controller.yaw -= delta.x.to_radians() * 0.5;
            controller.pitch -= delta.y.to_radians() * 0.5;
            controller.pitch = controller
                .pitch
                .clamp(-89.9f32.to_radians(), 89.9f32.to_radians());

            // Calculate movement in camera space
            let rotation = Quat::from_euler(EulerRot::YXZ, controller.yaw, controller.pitch, 0.0);
            let movement = rotation * move_vec * speed * time_delta_seconds;
            controller.position += movement;
            controller.position.y = controller.position.y.max(0.);
        }
        CameraType::Orbit => {
            let mut distance = controller.distance_to_target();

            for event in scroll_events.read() {
                distance -= speed * normalize_scroll_wheel(event) * scroll_sensitivity;
            }
            distance = distance.clamp(
                orbit_min_distance(*voxel_size_meters),
                orbit_max_distance(*voxel_size_meters),
            );

            // Calculate rotation from position to target
            let offset = controller.position - controller.target;
            let current_rotation = Quat::from_rotation_arc(Vec3::Z, offset.normalize());
            let (mut yaw, mut pitch, _) = current_rotation.to_euler(EulerRot::YXZ);

            yaw -= delta.x.to_radians();
            pitch -= delta.y.to_radians();
            pitch = pitch.clamp(-89.9f32.to_radians(), 89.9f32.to_radians());

            let rotation = Quat::from_euler(EulerRot::YXZ, yaw, pitch, 0.0);
            controller.position = controller.target + rotation.mul_vec3(Vec3::Z * distance);

            let position_delta = rotation * move_vec * speed * time_delta_seconds;
            controller.position += position_delta;
            controller.target += position_delta;

            controller.position.y = controller.position.y.max(0.);
            controller.target.y = controller.target.y.max(0.);
        }
    }
}

/// System that applies camera controller state to the actual Transform
fn apply_camera_controller(mut cameras: Query<(&CameraType, &CameraController, &mut Transform)>) {
    for (camera_type, controller, mut transform) in &mut cameras {
        transform.translation = controller.position;

        match camera_type {
            CameraType::Free => {
                transform.rotation =
                    Quat::from_euler(EulerRot::YXZ, controller.yaw, controller.pitch, 0.0);
            }
            CameraType::Orbit => {
                transform.look_at(controller.target, Vec3::Y);
            }
        }
    }
}

fn draw_orbit_camera_target(
    mut gizmos: Gizmos<MainCameraGizmos>,
    main_camera_query: Query<(&CameraType, &CameraController), With<MainCamera>>,
) {
    let Ok((camera_type, controller)) = main_camera_query.single() else {
        return;
    };

    if *camera_type != CameraType::Orbit {
        return;
    }

    let size = 0.5;
    let position = controller.target;
    draw_gizmo_cross(&mut gizmos, position, size);
}

fn sync_primary_and_secondary_camera_transforms(
    q_main: Query<&CameraController, (With<MainCamera>, Without<SecondaryCamera>)>,
    mut q_sec: Query<&mut CameraController, (With<SecondaryCamera>, Without<MainCamera>)>,
) {
    let Ok(controller_main) = q_main.single() else {
        return;
    };
    let Ok(mut controller_sec) = q_sec.single_mut() else {
        return;
    };

    controller_sec.position = controller_main.position;
    controller_sec.yaw = controller_main.yaw;
    controller_sec.pitch = controller_main.pitch;
    controller_sec.target = controller_main.target;
}
