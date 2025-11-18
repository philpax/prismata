use bevy::{
    input::mouse::{MouseMotion, MouseWheel},
    pbr::Atmosphere,
    prelude::*,
    render::view::RenderLayers,
};
use bevy_dolly::prelude::*;
use bevy_egui::egui;
use bevy_mod_raycast::prelude::Raycast;

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

#[derive(Component, PartialEq, Eq)]
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

pub fn plugin(app: &mut App) {
    app.add_systems(Startup, setup).add_systems(
        Update,
        (
            swap_camera,
            update_camera.run_if(is_cursor_invisible),
            sync_primary_and_secondary_camera_transforms,
            Dolly::<MainCamera>::update_active,
            Dolly::<SecondaryCamera>::update_active,
            draw_orbit_camera_target.run_if(is_cursor_invisible),
        )
            .chain(),
    );
}

pub fn ui_top_right_panel(
    ui: &mut egui::Ui,
    main_camera: Query<(&CameraType, &Rig), With<MainCamera>>,
) {
    let (camera_type, rig) = main_camera.single();
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Camera:").underline());
        ui.label(camera_type.to_string());
        if let (Some(position), Some(lookat)) =
            (rig.try_driver::<Position>(), rig.try_driver::<LookAt>())
        {
            let distance = position.position.distance(lookat.target);
            ui.label(format!("({distance:.2}m)"));
        }
    });
    ui.label("Press T to swap.");
}

fn setup(mut commands: Commands) {
    // Camera
    let target = Vec3::new(0., 0.45, 0.);
    let transform = Transform::from_xyz(-0.45, 0.45, -0.45).looking_at(target, Vec3::Y);
    let render_layers =
        RenderLayers::from_layers(&[ALL_NON_MASK_CAMERA_LAYER, MAIN_CAMERA_ONLY_LAYER]);
    commands.spawn((
        MainCamera,
        CameraType::Free,
        Rig::builder()
            .with(Fpv::from_position_target(transform))
            .build(),
        Camera3dBundle {
            transform,
            ..default()
        },
        #[cfg(feature = "webgpu")]
        Atmosphere::EARTH,
        render_layers.clone(),
        transform_gizmo_bevy::GizmoCamera,
    ));
    commands.spawn((
        SecondaryCamera,
        CameraType::Orbit,
        Rig::builder()
            .with(Position::new(target))
            .with(LookAt::new(target))
            .build(),
        Camera3dBundle {
            camera: Camera {
                is_active: false,
                ..default()
            },
            transform,
            ..default()
        },
        #[cfg(feature = "webgpu")]
        Atmosphere::EARTH,
        render_layers,
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
    mut world_raycast: Raycast,
    mut commands: Commands,
    mut q_main: Query<
        (Entity, &mut Camera, &GlobalTransform),
        (With<MainCamera>, Without<SecondaryCamera>),
    >,
    mut q_sec: Query<
        (Entity, &mut Camera, &CameraType, &mut Rig),
        (With<SecondaryCamera>, Without<MainCamera>),
    >,

    chunk_datas: Query<&ChunkData>,
    raycast_ignores: Query<(), With<RaycastIgnore>>,
    parents: Query<&Parent>,

    chunks: Res<Chunks>,
    voxels_per_meter: Res<VoxelsPerMeter>,
    voxel_size_meters: Res<VoxelSizeMeters>,
) {
    if !keys.just_pressed(KeyCode::KeyT) {
        return;
    }
    let Ok((e_main, mut cam_main, transform_main)) = q_main.get_single_mut() else {
        return;
    };
    let Ok((e_sec, mut cam_sec, type_sec, mut rig_sec)) = q_sec.get_single_mut() else {
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
            Ray3d::new(origin, direction),
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
        rig_sec.driver_mut::<LookAt>().target = target;
    }
}

pub fn update_camera(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    voxel_size_meters: Res<VoxelSizeMeters>,
    mut mouse_motion_events: EventReader<MouseMotion>,
    mut scroll_events: EventReader<MouseWheel>,
    mut main_camera_query: Query<(&mut Rig, &GlobalTransform), With<MainCamera>>,
) {
    let time_delta_seconds: f32 = time.delta_seconds();
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

    let (mut rig, transform) = main_camera_query.single_mut();

    if let Some(rig) = rig.try_driver_mut::<Fpv>() {
        rig.update_pos_rot(move_vec, delta, false, speed, time_delta_seconds);
        let position = &mut rig.driver_mut::<Position>().position;
        position.y = position.y.max(0.);
    } else {
        let mut position = rig.driver::<Position>().position;
        let mut target = rig.driver::<LookAt>().target;
        let mut distance = position.distance(target);
        let rotation = transform.to_scale_rotation_translation().1;

        for event in scroll_events.read() {
            distance -= speed * normalize_scroll_wheel(event) * scroll_sensitivity;
        }
        distance = distance.clamp(
            orbit_min_distance(*voxel_size_meters),
            orbit_max_distance(*voxel_size_meters),
        );

        let (mut yaw, mut pitch, _) = rotation.to_euler(EulerRot::YXZ);
        yaw -= delta.x.to_radians();
        pitch -= delta.y.to_radians();
        pitch = pitch.clamp(-89.9f32.to_radians(), 89.9f32.to_radians());
        let rotation = Quat::from_euler(EulerRot::YXZ, yaw, pitch, 0.0);
        position = target + rotation.mul_vec3(Vec3::Z * distance);

        let position_delta = rotation * move_vec * speed * time_delta_seconds;
        position += position_delta;
        target += position_delta;

        position.y = position.y.max(0.);
        target.y = target.y.max(0.);

        rig.driver_mut::<Position>().position = position;
        rig.driver_mut::<LookAt>().target = target;
    }
}

fn draw_orbit_camera_target(
    mut gizmos: Gizmos<MainCameraGizmos>,
    main_camera_query: Query<&Rig, With<MainCamera>>,
) {
    let Some(look_at) = main_camera_query
        .get_single()
        .ok()
        .and_then(|r| r.try_driver::<LookAt>())
    else {
        return;
    };

    let size = 0.5;
    let position = look_at.target;
    draw_gizmo_cross(&mut gizmos, position, size);
}

fn sync_primary_and_secondary_camera_transforms(
    q_main: Query<(&Rig, &GlobalTransform), (With<MainCamera>, Without<SecondaryCamera>)>,
    mut q_sec: Query<&mut Rig, (With<SecondaryCamera>, Without<MainCamera>)>,
) {
    let Ok((rig_main, transform_main)) = q_main.get_single() else {
        return;
    };
    let Ok(mut rig_sec) = q_sec.get_single_mut() else {
        return;
    };
    if let Some((position, rotation)) = get_rig_position_rotation(rig_main, transform_main) {
        set_rig_position_rotation(&mut rig_sec, position, rotation);
    }
}

fn get_rig_position_rotation(rig: &Rig, transform: &GlobalTransform) -> Option<(Vec3, (f32, f32))> {
    let rig = rig
        .try_driver::<Fpv>()
        .map(|r| r as &CameraRig)
        .unwrap_or(rig);
    Some((
        rig.try_driver::<Position>()?.position,
        rig.try_driver::<YawPitch>()
            .map(|yp| (yp.yaw_degrees, yp.pitch_degrees))
            .unwrap_or_else(|| {
                let rotation = transform
                    .to_scale_rotation_translation()
                    .1
                    .to_euler(EulerRot::YXZ);
                (rotation.0.to_degrees(), rotation.1.to_degrees())
            }),
    ))
}

fn set_rig_position_rotation(rig: &mut Rig, position: Vec3, rotation: (f32, f32)) {
    let rig = if let Some(rig) = rig.try_driver_mut::<Fpv>() {
        rig as &mut CameraRig
    } else {
        rig
    };
    if let Some(position_driver) = rig.try_driver_mut::<Position>() {
        position_driver.position = position;
    }
    if let Some(yaw_pitch_driver) = rig.try_driver_mut::<YawPitch>() {
        yaw_pitch_driver.yaw_degrees = rotation.0;
        yaw_pitch_driver.pitch_degrees = rotation.1;
    }
}
