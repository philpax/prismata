use bevy::{camera::visibility::RenderLayers, input::mouse::MouseMotion, prelude::*};
use bevy_egui::egui;

use crate::{
    raycast::{update_world_rayhits, CursorRayHitWithoutDraft, RaycastIgnore},
    rendering::{
        draw_gizmo_cross, AlphaPulse, MainCameraGizmosWithoutDepth, MAIN_CAMERA_ONLY_LAYER,
    },
    ui::{is_cursor_visible, CursorVisible},
    voxel::{self, Coords},
};

use super::{
    brush_should_be_created, brush_should_be_destroyed, brush_size_range, scroll_size_system,
    stopped_being_used, LastUsedColors, Tool, ToolColor,
};

#[derive(Resource)]
pub struct OurWallBrush(pub Entity);

#[derive(Resource)]
pub struct OurWallBrushSettings {
    pub thickness: f32,
}

#[derive(Resource, Copy, Clone)]
pub enum WallBrushState {
    WaitingForPoint1,
    Point1 {
        point1: Coords,
    },
    Ready {
        point1: Coords,
        point2: Coords,
        height: f32,
    },
}

pub fn plugin(app: &mut App) {
    app.insert_resource(OurWallBrushSettings { thickness: 0.0 })
        .add_systems(
            Update,
            (
                scroll_size_system(Tool::Wall, |s: &mut OurWallBrushSettings| &mut s.thickness),
                create_brush.run_if(brush_should_be_created::<OurWallBrush>(Tool::Wall)),
                destroy_brush.run_if(brush_should_be_destroyed::<OurWallBrush>(Tool::Wall)),
                update_brush_height
                    .run_if(resource_exists::<WallBrushState>.and(is_cursor_visible)),
                update_brush_viz
                    .run_if(resource_exists::<OurWallBrush>.and(resource_exists::<WallBrushState>)),
                on_click
                    .run_if(stopped_being_used(Tool::Wall).and(resource_exists::<WallBrushState>)),
            )
                .chain()
                .after(update_world_rayhits),
        );
}

pub fn ui_top_left_panel(ui: &mut egui::Ui, world: &mut World) {
    let Some(&state) = world.get_resource::<WallBrushState>() else {
        return;
    };
    let range = brush_size_range(world);
    let mut settings = world.resource_mut::<OurWallBrushSettings>();
    ui.add(
        egui::Slider::new(&mut settings.thickness, range)
            .fixed_decimals(2)
            .text("Thickness"),
    );
    ui.separator();
    ui.label(match state {
        WallBrushState::WaitingForPoint1 => "Click to start creating a wall.".to_string(),
        WallBrushState::Point1 { .. } => "Click to set the second point of the wall.".to_string(),
        WallBrushState::Ready { height, .. } => format!("Height: {height:.2}m"),
    });
}

fn create_brush(
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut commands: Commands,
) {
    let id = commands
        .spawn((
            Name::new("Wall Brush"),
            Mesh3d(
                meshes.add(
                    Cuboid {
                        half_size: Vec3::splat(0.5),
                    }
                    .mesh(),
                ),
            ),
            MeshMaterial3d(materials.add(StandardMaterial {
                alpha_mode: AlphaMode::Blend,
                base_color: Color::linear_rgba(1.0, 1.0, 1.0, 1.0),
                unlit: true,
                ..default()
            })),
            Transform::default(),
            Visibility::Hidden,
            AlphaPulse::new(0.25, 1.0),
            RenderLayers::layer(MAIN_CAMERA_ONLY_LAYER as usize),
            RaycastIgnore,
        ))
        .id();
    commands.insert_resource(OurWallBrush(id));
    commands.insert_resource(WallBrushState::WaitingForPoint1);
}

fn destroy_brush(brush: Res<OurWallBrush>, mut commands: Commands) {
    commands.entity(brush.0).despawn();
    commands.remove_resource::<OurWallBrush>();
    commands.remove_resource::<WallBrushState>();
}

fn update_brush_height(
    mut state: ResMut<WallBrushState>,
    mut mouse_motion_events: MessageReader<MouseMotion>,
) {
    if let WallBrushState::Ready { height, .. } = &mut *state {
        // TODO: consider going for motion in the direction of the extrusion, not just pure vertical movement
        *height -= mouse_motion_events.read().map(|e| e.delta.y).sum::<f32>() * 0.002;
    }
}

fn update_brush_viz(
    voxel_under_cursor: Res<CursorRayHitWithoutDraft>,
    cursor_visible: Res<CursorVisible>,
    tool_color: Res<ToolColor>,
    voxels_per_meter: Res<voxel::VoxelsPerMeter>,
    our_wall_brush: Res<OurWallBrush>,
    settings: Res<OurWallBrushSettings>,
    state: Res<WallBrushState>,

    mut gizmos: Gizmos<MainCameraGizmosWithoutDepth>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut tool_brushes: Query<(
        &mut Transform,
        &mut Visibility,
        &MeshMaterial3d<StandardMaterial>,
    )>,
) {
    let (mut transform, mut visibility, color) = tool_brushes.get_mut(our_wall_brush.0).unwrap();

    materials.get_mut(&color.0).unwrap().base_color = tool_color.base;
    match *state {
        WallBrushState::WaitingForPoint1 => {
            if let Some(hit) = voxel_under_cursor.ray_hit.filter(|_| cursor_visible.0) {
                transform.translation = hit.entry_coords.to_world(*voxels_per_meter)
                    + Vec3::Y * settings.thickness / 2.0;
                transform.rotation = Quat::IDENTITY;
                transform.scale = Vec3::splat(settings.thickness);
                *visibility = Visibility::Visible;
            } else {
                *visibility = Visibility::Hidden;
            }
        }
        WallBrushState::Point1 { point1 } => {
            if let Some(hit) = voxel_under_cursor.ray_hit.filter(|_| cursor_visible.0) {
                *visibility = Visibility::Visible;
                *transform = calculate_transform_from_points(
                    &mut gizmos,
                    *voxels_per_meter,
                    point1,
                    hit.entry_coords,
                    settings.thickness,
                    settings.thickness,
                );
            } else {
                *visibility = Visibility::Hidden;
            }
        }
        WallBrushState::Ready {
            point1,
            point2,
            height,
        } => {
            *visibility = Visibility::Visible;
            *transform = calculate_transform_from_points(
                &mut gizmos,
                *voxels_per_meter,
                point1,
                point2,
                settings.thickness,
                height,
            );
        }
    }
}

fn calculate_transform_from_points(
    gizmos: &mut Gizmos<MainCameraGizmosWithoutDepth>,
    voxels_per_meter: voxel::VoxelsPerMeter,
    point1: Coords,
    point2: Coords,
    thickness: f32,
    height: f32,
) -> Transform {
    let point1 = point1.to_world(voxels_per_meter);
    let point2 = point2.to_world(voxels_per_meter);
    let delta = point2 - point1;
    let distance = delta.length();
    let forward = delta / distance;

    // Calculate right vector (perpendicular to forward and global up)
    let right = Vec3::Y.cross(forward).normalize();

    // Recalculate up vector to ensure it's perpendicular to forward and right
    let up = forward.cross(right).normalize();

    let midpoint = (point1 + point2) / 2.0;
    let center = midpoint + up * (height / 2.0);

    draw_gizmo_cross(gizmos, point1, thickness);
    draw_gizmo_cross(gizmos, point2, thickness);
    draw_gizmo_cross(gizmos, midpoint, thickness);
    gizmos.line(point1, point2, Color::WHITE);
    gizmos.line(midpoint, midpoint + up * height, Color::WHITE);

    // Create rotation from the orthonormal basis
    let rotation = Quat::from_mat3(&Mat3::from_cols(right, up, forward));

    Transform {
        translation: center,
        rotation,
        scale: Vec3::new(thickness, height.abs(), distance),
    }
}

fn on_click(
    cursor_ray_hit_without_draft: Res<CursorRayHitWithoutDraft>,
    settings: Res<OurWallBrushSettings>,
    tool_color: Res<ToolColor>,
    voxels_per_meter: Res<voxel::VoxelsPerMeter>,
    mut state: ResMut<WallBrushState>,
    mut last_used_colors: ResMut<LastUsedColors>,
    mut pending_dynamic_updates: MessageWriter<voxel::ChunkPendingDynamicUpdate>,
) {
    let voxels_per_meter = *voxels_per_meter;
    match &mut *state {
        WallBrushState::WaitingForPoint1 => {
            if let Some(target_coords) = cursor_ray_hit_without_draft.coords() {
                *state = WallBrushState::Point1 {
                    point1: target_coords,
                };
            }
        }
        WallBrushState::Point1 { point1 } => {
            if let Some(target_coords) = cursor_ray_hit_without_draft.coords() {
                *state = WallBrushState::Ready {
                    point1: *point1,
                    point2: target_coords,
                    height: settings.thickness,
                };
            }
        }
        WallBrushState::Ready {
            point1,
            point2,
            height,
        } => {
            last_used_colors.push(tool_color.base);
            let prism = WallPrism::new(
                point1.to_world(voxels_per_meter),
                point2.to_world(voxels_per_meter),
                *height,
                settings.thickness,
            );
            let color = *tool_color;

            // TODO: optimise this, we only need to handle chunks along the wall, but this will
            // materialise the full cube of chunks around the locaiton
            voxel::update(
                &mut pending_dynamic_updates,
                voxels_per_meter,
                voxel::UpdateParams {
                    center: Coords::from_world(prism.center, voxels_per_meter),
                    // fudge factor for corners
                    distance_meters: settings.thickness.max(prism.longest_axis())
                        + 2.0 * settings.thickness,
                    should_allocate_chunk: move |c, cs| {
                        prism.signed_distance(c.to_world(voxels_per_meter)) <= cs
                    },
                    update: move |coords, old_voxel| {
                        if prism.contains_point(coords.to_world(voxels_per_meter)) {
                            voxel::Voxel::new(color.sample(), voxel::VoxelMaterial::Draft)
                        } else {
                            old_voxel
                        }
                    },
                },
            );
            *state = WallBrushState::WaitingForPoint1;
        }
    }
}

#[derive(Copy, Clone)]
struct WallPrism {
    center: Vec3,
    right: Vec3,
    up: Vec3,
    forward: Vec3,
    half_thickness: f32,
    half_height: f32,
    half_length: f32,
}
impl WallPrism {
    fn new(point1: Vec3, point2: Vec3, height: f32, thickness: f32) -> Self {
        let forward = (point2 - point1).normalize();
        let up = Vec3::Y;
        let right = forward.cross(up).normalize();
        let up = right.cross(forward); // Recalculate to ensure orthogonality

        let center = (point1 + point2) * 0.5 + up * (height * 0.5);
        let half_length = (point2 - point1).length() * 0.5;

        WallPrism {
            center,
            right,
            up,
            forward,
            half_thickness: thickness * 0.5,
            half_height: (height * 0.5).abs(),
            half_length,
        }
    }
    fn contains_point(&self, point: Vec3) -> bool {
        let local_point = point - self.center;
        let local_x = local_point.dot(self.right);
        let local_y = local_point.dot(self.up);
        let local_z = local_point.dot(self.forward);

        local_x.abs() <= self.half_thickness
            && local_y.abs() <= self.half_height
            && local_z.abs() <= self.half_length
    }
    fn half_size(&self) -> Vec3 {
        Vec3::new(self.half_thickness, self.half_height, self.half_length)
    }
    fn longest_axis(&self) -> f32 {
        self.half_size().max_element()
    }
    fn signed_distance(&self, point: Vec3) -> f32 {
        let local_point = point - self.center;
        let local_coords = Vec3::new(
            local_point.dot(self.right),
            local_point.dot(self.up),
            local_point.dot(self.forward),
        );

        let half_size = Vec3::new(self.half_thickness, self.half_height, self.half_length);
        let d = local_coords.abs() - half_size;

        d.max(Vec3::ZERO).length() + d.x.max(d.y.max(d.z)).min(0.0)
    }
}
