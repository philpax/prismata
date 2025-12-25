use web_time::Duration;

use bevy::{camera::visibility::RenderLayers, picking::prelude::Pickable, prelude::*};
use bevy_egui::egui;

use crate::{
    raycast::{update_world_rayhits, CursorRayHitWithoutDraft, RaycastIgnore},
    rendering::{AlphaPulse, MAIN_CAMERA_ONLY_LAYER},
    ui::CursorVisible,
    voxel,
};

use super::{
    brush_should_be_created, brush_should_be_destroyed, brush_size_range, is_in_use,
    scroll_size_system, LastUsedColors, Tool, ToolColor,
};

#[derive(Resource)]
pub struct OurSphereBrush(pub Entity);

#[derive(Resource)]
pub struct OurSphereBrushSettings {
    pub radius: f32,
}

pub fn plugin(app: &mut App) {
    app.insert_resource(OurSphereBrushSettings { radius: 0.0 })
        .add_systems(
            Update,
            (
                scroll_size_system(Tool::Sphere, |s: &mut OurSphereBrushSettings| &mut s.radius),
                create_brush.run_if(brush_should_be_created::<OurSphereBrush>(Tool::Sphere)),
                destroy_brush.run_if(brush_should_be_destroyed::<OurSphereBrush>(Tool::Sphere)),
                update_brush_viz.run_if(resource_exists::<OurSphereBrush>),
                on_use.run_if(is_in_use(Tool::Sphere, Some(Duration::from_millis(30)))),
            )
                .chain()
                .after(update_world_rayhits),
        );
}

fn create_brush(
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut commands: Commands,
) {
    let id = commands
        .spawn((
            Name::new("Sphere Brush"),
            Mesh3d(meshes.add(Sphere { radius: 1.0 }.mesh().ico(5).unwrap())),
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
    commands.insert_resource(OurSphereBrush(id));
}

fn destroy_brush(brush: Res<OurSphereBrush>, mut commands: Commands) {
    commands.entity(brush.0).despawn();
    commands.remove_resource::<OurSphereBrush>();
}

fn update_brush_viz(
    voxel_under_cursor: Res<CursorRayHitWithoutDraft>,
    cursor_visible: Res<CursorVisible>,
    tool_color: Res<ToolColor>,
    voxels_per_meter: Res<voxel::VoxelsPerMeter>,
    our_sphere_brush: Res<OurSphereBrush>,
    settings: Res<OurSphereBrushSettings>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut tool_brushes: Query<(
        &mut Transform,
        &mut Visibility,
        &MeshMaterial3d<StandardMaterial>,
    )>,
) {
    let (mut transform, mut visibility, color) = tool_brushes.get_mut(our_sphere_brush.0).unwrap();
    if let Some(voxel) = voxel_under_cursor.ray_hit.filter(|_| cursor_visible.0) {
        transform.translation = voxel.entry_coords.to_world(*voxels_per_meter);
        transform.scale = Vec3::splat(settings.radius);
        *visibility = Visibility::Visible;
        materials.get_mut(&color.0).unwrap().base_color = tool_color.base;
    } else {
        *visibility = Visibility::Hidden;
    }
}

fn on_use(
    cursor_ray_hit_without_draft: Res<CursorRayHitWithoutDraft>,
    settings: Res<OurSphereBrushSettings>,
    tool_color: Res<ToolColor>,
    voxels_per_meter: Res<voxel::VoxelsPerMeter>,
    mut last_used_colors: ResMut<LastUsedColors>,
    mut pending_dynamic_updates: MessageWriter<voxel::ChunkPendingDynamicUpdate>,
) {
    let Some(center) = cursor_ray_hit_without_draft.coords() else {
        return;
    };

    last_used_colors.push(tool_color.base);
    let radius = settings.radius;
    let radius_voxel = (radius * voxels_per_meter.0 as f32) as u32;
    let radius_voxels_sqr = radius_voxel * radius_voxel;
    let color = *tool_color;

    voxel::update(
        &mut pending_dynamic_updates,
        *voxels_per_meter,
        voxel::UpdateParams {
            center,
            distance_meters: radius,
            should_allocate_chunk: |_, _| true,
            update: move |coords, old_voxel| {
                let distance_voxels_sqr = center.distance_squared(coords);
                if distance_voxels_sqr >= radius_voxels_sqr {
                    return old_voxel;
                }

                let tool_color = color.sample();
                if old_voxel.material == voxel::VoxelMaterial::Air {
                    voxel::Voxel::new(tool_color, voxel::VoxelMaterial::Draft)
                } else {
                    let pos_factor = 1.0 - (distance_voxels_sqr as f32 / radius_voxels_sqr as f32);
                    old_voxel
                        .with_mixed_color(tool_color, pos_factor)
                        .with_material(voxel::VoxelMaterial::Draft)
                }
            },
        },
    );
}

pub fn ui_top_left_panel(ui: &mut egui::Ui, world: &mut World) {
    let range = brush_size_range(world);
    let mut settings = world.resource_mut::<OurSphereBrushSettings>();
    ui.add(
        egui::Slider::new(&mut settings.radius, range)
            .fixed_decimals(2)
            .text("Radius"),
    );
}
