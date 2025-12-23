use web_time::Duration;

use bevy::{camera::visibility::RenderLayers, prelude::*};
use bevy_egui::egui;

use crate::{
    raycast::{update_world_rayhits, CursorRayHitWithoutDraft, RaycastIgnore},
    rendering::{AlphaPulse, MAIN_CAMERA_ONLY_LAYER},
    ui::CursorVisible,
    voxel::{self, Coords},
};

use super::{
    brush_should_be_created, brush_should_be_destroyed, brush_size_range, is_in_use,
    scroll_size_system, LastUsedColors, Tool, ToolColor,
};

#[derive(Resource)]
pub struct OurCubeBrush(pub Entity);

#[derive(Resource)]
pub struct OurCubeBrushSettings {
    pub size: f32,
}

pub fn plugin(app: &mut App) {
    app.insert_resource(OurCubeBrushSettings { size: 0.0 })
        .add_systems(
            Update,
            (
                scroll_size_system(Tool::Cube, |s: &mut OurCubeBrushSettings| &mut s.size),
                create_brush.run_if(brush_should_be_created::<OurCubeBrush>(Tool::Cube)),
                destroy_brush.run_if(brush_should_be_destroyed::<OurCubeBrush>(Tool::Cube)),
                update_brush_viz.run_if(resource_exists::<OurCubeBrush>),
                on_use.run_if(is_in_use(Tool::Cube, Some(Duration::from_millis(30)))),
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
            Name::new("Cube Brush"),
            Mesh3d(
                meshes.add(
                    Cuboid {
                        half_size: Vec3::splat(1.0),
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
    commands.insert_resource(OurCubeBrush(id));
}

fn destroy_brush(brush: Res<OurCubeBrush>, mut commands: Commands) {
    commands.entity(brush.0).despawn();
    commands.remove_resource::<OurCubeBrush>();
}

fn update_brush_viz(
    voxel_under_cursor: Res<CursorRayHitWithoutDraft>,
    cursor_visible: Res<CursorVisible>,
    tool_color: Res<ToolColor>,
    voxels_per_meter: Res<voxel::VoxelsPerMeter>,
    our_cube_brush: Res<OurCubeBrush>,
    settings: Res<OurCubeBrushSettings>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut tool_brushes: Query<(
        &mut Transform,
        &mut Visibility,
        &MeshMaterial3d<StandardMaterial>,
    )>,
) {
    let (mut transform, mut visibility, color) = tool_brushes.get_mut(our_cube_brush.0).unwrap();
    if let Some(voxel) = voxel_under_cursor.ray_hit.filter(|_| cursor_visible.0) {
        transform.translation =
            voxel.entry_coords.to_world(*voxels_per_meter) + Vec3::Y * settings.size;
        transform.scale = Vec3::splat(settings.size);
        *visibility = Visibility::Visible;
        materials.get_mut(&color.0).unwrap().base_color = tool_color.base;
    } else {
        *visibility = Visibility::Hidden;
    }
}

fn on_use(
    cursor_ray_hit_without_draft: Res<CursorRayHitWithoutDraft>,
    settings: Res<OurCubeBrushSettings>,
    tool_color: Res<ToolColor>,
    voxels_per_meter: Res<voxel::VoxelsPerMeter>,
    mut last_used_colors: ResMut<LastUsedColors>,
    mut pending_dynamic_updates: MessageWriter<voxel::ChunkPendingDynamicUpdate>,
) {
    let Some(target_coords) = cursor_ray_hit_without_draft.coords() else {
        return;
    };

    let target_coords_offset_pos =
        target_coords.to_world(*voxels_per_meter) + Vec3::Y * settings.size;
    let center = Coords::from_world(target_coords_offset_pos, *voxels_per_meter);

    last_used_colors.push(tool_color.base);

    let color = *tool_color;
    voxel::update(
        &mut pending_dynamic_updates,
        *voxels_per_meter,
        voxel::UpdateParams {
            center,
            distance_meters: settings.size,
            should_allocate_chunk: |_, _| true,
            update: move |_coords, _old_voxel| {
                voxel::Voxel::new(color.sample(), voxel::VoxelMaterial::Draft)
            },
        },
    );
}

pub fn ui_top_left_panel(ui: &mut egui::Ui, world: &mut World) {
    let range = brush_size_range(world);
    let mut settings = world.resource_mut::<OurCubeBrushSettings>();
    ui.add(
        egui::Slider::new(&mut settings.size, range)
            .fixed_decimals(2)
            .text("Size"),
    );
}
