use bevy::{
    camera::visibility::RenderLayers, input::mouse::MouseMotion, picking::prelude::Pickable,
    prelude::*,
};
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
pub struct OurFloorBrush(pub Entity);

#[derive(Resource)]
pub struct OurFloorBrushSettings {
    pub thickness: f32,
}

#[derive(Resource, Copy, Clone)]
pub enum FloorBrushState {
    WaitingForPoint1,
    Point1 {
        point1: Coords,
    },
    Ready {
        point1: Coords,
        point2: Coords,
        width: f32,
    },
}

pub fn plugin(app: &mut App) {
    app.insert_resource(OurFloorBrushSettings { thickness: 0.0 })
        .add_systems(
            Update,
            (
                scroll_size_system(Tool::Floor, |s: &mut OurFloorBrushSettings| {
                    &mut s.thickness
                }),
                create_brush.run_if(brush_should_be_created::<OurFloorBrush>(Tool::Floor)),
                destroy_brush.run_if(brush_should_be_destroyed::<OurFloorBrush>(Tool::Floor)),
                update_floor_width
                    .run_if(resource_exists::<FloorBrushState>.and(is_cursor_visible)),
                update_brush_viz.run_if(
                    resource_exists::<OurFloorBrush>.and(resource_exists::<FloorBrushState>),
                ),
                on_click.run_if(
                    stopped_being_used(Tool::Floor).and(resource_exists::<FloorBrushState>),
                ),
            )
                .chain()
                .after(update_world_rayhits),
        );
}

pub fn ui_top_left_panel(ui: &mut egui::Ui, world: &mut World) {
    let Some(&state) = world.get_resource::<FloorBrushState>() else {
        return;
    };
    let range = brush_size_range(world);
    let mut settings = world.resource_mut::<OurFloorBrushSettings>();
    ui.add(
        egui::Slider::new(&mut settings.thickness, range)
            .fixed_decimals(2)
            .text("Thickness"),
    );
    ui.separator();
    ui.label(match state {
        FloorBrushState::WaitingForPoint1 => "Click to start creating a floor.".to_string(),
        FloorBrushState::Point1 { .. } => {
            "Click to set the second point of the floor edge.".to_string()
        }
        FloorBrushState::Ready { width, .. } => format!("Width: {width:.2}m"),
    });
}

fn create_brush(
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut commands: Commands,
) {
    let id = commands
        .spawn((
            Name::new("Floor Brush"),
            Mesh3d(meshes.add(Mesh::from(Plane3d::new(Vec3::Y, Vec2::ONE * 0.5)))),
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
            Pickable::IGNORE,
        ))
        .id();
    commands.insert_resource(OurFloorBrush(id));
    commands.insert_resource(FloorBrushState::WaitingForPoint1);
}

fn destroy_brush(brush: Res<OurFloorBrush>, mut commands: Commands) {
    commands.entity(brush.0).despawn();
    commands.remove_resource::<OurFloorBrush>();
    commands.remove_resource::<FloorBrushState>();
}

fn update_floor_width(
    mut state: ResMut<FloorBrushState>,
    mut mouse_motion_events: MessageReader<MouseMotion>,
) {
    if let FloorBrushState::Ready { width, .. } = &mut *state {
        *width += mouse_motion_events.read().map(|e| e.delta.x).sum::<f32>() * 0.002;
    }
}

fn update_brush_viz(
    voxel_under_cursor: Res<CursorRayHitWithoutDraft>,
    cursor_visible: Res<CursorVisible>,
    tool_color: Res<ToolColor>,
    voxels_per_meter: Res<voxel::VoxelsPerMeter>,
    our_floor_brush: Res<OurFloorBrush>,
    settings: Res<OurFloorBrushSettings>,
    state: Res<FloorBrushState>,
    mut gizmos: Gizmos<MainCameraGizmosWithoutDepth>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut tool_brushes: Query<(
        &mut Transform,
        &mut Visibility,
        &MeshMaterial3d<StandardMaterial>,
    )>,
) {
    let (mut transform, mut visibility, color) = tool_brushes.get_mut(our_floor_brush.0).unwrap();

    materials.get_mut(&color.0).unwrap().base_color = tool_color.base;
    match *state {
        FloorBrushState::WaitingForPoint1 => {
            if let Some(hit) = voxel_under_cursor.ray_hit.filter(|_| cursor_visible.0) {
                transform.translation = hit.entry_coords.to_world(*voxels_per_meter)
                    + Vec3::Y * settings.thickness / 2.0;
                transform.rotation = Quat::IDENTITY;
                transform.scale =
                    Vec3::new(settings.thickness, settings.thickness, settings.thickness);
                *visibility = Visibility::Visible;
            } else {
                *visibility = Visibility::Hidden;
            }
        }
        FloorBrushState::Point1 { point1 } => {
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
        FloorBrushState::Ready {
            point1,
            point2,
            width,
        } => {
            *visibility = Visibility::Visible;
            *transform = calculate_transform_from_points(
                &mut gizmos,
                *voxels_per_meter,
                point1,
                point2,
                settings.thickness,
                width,
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
    width: f32,
) -> Transform {
    let point1 = point1.to_world(voxels_per_meter);
    let point2 = point2.to_world(voxels_per_meter);
    let delta = point2 - point1;
    let length = delta.length();
    let forward = delta / length;

    // Calculate right vector (perpendicular to forward and global up)
    let global_up = Vec3::Y;
    let right = forward.cross(global_up).normalize();

    // Recalculate up vector to be perpendicular to forward and right
    let up = right.cross(forward).normalize();

    let midpoint = (point1 + point2) / 2.0;
    let center = midpoint + right * (-width / 2.0) + Vec3::Y * thickness / 2.0;

    // Drawing gizmos...
    draw_gizmo_cross(gizmos, point1, thickness);
    draw_gizmo_cross(gizmos, point2, thickness);
    draw_gizmo_cross(gizmos, midpoint, thickness);
    gizmos.line(point1, point2, Color::WHITE);
    gizmos.line(midpoint, center, Color::WHITE);

    // Create rotation from the orthonormal basis
    let rotation = Quat::from_mat3(&Mat3::from_cols(forward, up, right));

    Transform {
        translation: center,
        rotation,
        scale: Vec3::new(length, thickness, width.abs()),
    }
}

fn on_click(
    cursor_ray_hit_without_draft: Res<CursorRayHitWithoutDraft>,
    settings: Res<OurFloorBrushSettings>,
    tool_color: Res<ToolColor>,
    voxels_per_meter: Res<voxel::VoxelsPerMeter>,
    mut state: ResMut<FloorBrushState>,
    mut last_used_colors: ResMut<LastUsedColors>,
    mut pending_dynamic_updates: MessageWriter<voxel::ChunkPendingDynamicUpdate>,
) {
    let voxels_per_meter = *voxels_per_meter;
    match &mut *state {
        FloorBrushState::WaitingForPoint1 => {
            if let Some(target_coords) = cursor_ray_hit_without_draft.coords() {
                *state = FloorBrushState::Point1 {
                    point1: target_coords,
                };
            }
        }
        FloorBrushState::Point1 { point1 } => {
            if let Some(target_coords) = cursor_ray_hit_without_draft.coords() {
                *state = FloorBrushState::Ready {
                    point1: *point1,
                    point2: target_coords,
                    width: settings.thickness,
                };
            }
        }
        FloorBrushState::Ready {
            point1,
            point2,
            width,
        } => {
            last_used_colors.push(tool_color.base);
            let prism = FloorPrism::new(
                point1.to_world(voxels_per_meter),
                point2.to_world(voxels_per_meter),
                *width,
                settings.thickness,
            );
            let color = *tool_color;

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
            *state = FloorBrushState::WaitingForPoint1;
        }
    }
}

#[derive(Copy, Clone)]
struct FloorPrism {
    center: Vec3,
    forward: Vec3,
    up: Vec3,
    right: Vec3,
    half_length: f32,
    half_width: f32,
    half_thickness: f32,
}
impl FloorPrism {
    fn new(point1: Vec3, point2: Vec3, width: f32, thickness: f32) -> Self {
        let forward = (point2 - point1).normalize();
        let global_up = Vec3::Y;
        let right = forward.cross(global_up).normalize();
        let up = right.cross(forward).normalize();

        let center = (point1 + point2) * 0.5 + right * (-width * 0.5);
        let half_length = (point2 - point1).length() * 0.5;

        FloorPrism {
            center,
            forward,
            up,
            right,
            half_length,
            half_width: (width * 0.5).abs(),
            half_thickness: thickness * 0.5,
        }
    }
    fn contains_point(&self, point: Vec3) -> bool {
        let local_point = point - self.center;
        let local_x = local_point.dot(self.forward);
        let local_y = local_point.dot(self.up);
        let local_z = local_point.dot(self.right);

        local_x.abs() <= self.half_length
            && local_y.abs() <= self.half_thickness
            && local_z.abs() <= self.half_width
    }
    fn half_size(&self) -> Vec3 {
        Vec3::new(self.half_length, self.half_thickness, self.half_width)
    }
    fn longest_axis(&self) -> f32 {
        self.half_size().max_element()
    }
    fn signed_distance(&self, point: Vec3) -> f32 {
        let local_point = point - self.center;
        let local_coords = Vec3::new(
            local_point.dot(self.forward),
            local_point.dot(self.up),
            local_point.dot(self.right),
        );

        let half_size = Vec3::new(self.half_length, self.half_thickness, self.half_width);
        let d = local_coords.abs() - half_size;

        d.max(Vec3::ZERO).length() + d.x.max(d.y.max(d.z)).min(0.0)
    }
}
