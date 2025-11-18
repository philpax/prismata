use bevy::{prelude::*, render::view::RenderLayers};
use bevy_egui::egui;

use crate::{
    color,
    raycast::{update_world_rayhits, CursorRayHit, RaycastIgnore},
    rendering::{AlphaPulse, MAIN_CAMERA_ONLY_LAYER},
    ui::CursorVisible,
    voxel,
};

use super::{
    brush_should_be_created, brush_should_be_destroyed, is_in_use, LastUsedColors, Tool, ToolColor,
};

#[derive(Resource)]
pub struct OurColorPickerBrush(pub Entity);

pub fn plugin(app: &mut App) {
    app.add_systems(
        Update,
        (
            create_brush.run_if(brush_should_be_created::<OurColorPickerBrush>(
                Tool::ColorPicker,
            )),
            destroy_brush.run_if(brush_should_be_destroyed::<OurColorPickerBrush>(
                Tool::ColorPicker,
            )),
            update_brush_viz.run_if(resource_exists::<OurColorPickerBrush>),
            on_use.run_if(is_in_use(Tool::ColorPicker, None)),
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
            Name::new("Color Picker Brush"),
            Mesh3d(meshes.add(Sphere { radius: 1.0 }.mesh().ico(5).unwrap())),
            MeshMaterial3d(materials.add(StandardMaterial {
                alpha_mode: AlphaMode::Blend,
                base_color: Color::linear_rgba(1.0, 1.0, 1.0, 1.0),
                unlit: true,
                ..default()
            })),
            Transform::default(),
            Visibility::Hidden,
            AlphaPulse::new(0.25, 0.5),
            RenderLayers::layer(MAIN_CAMERA_ONLY_LAYER),
            RaycastIgnore,
        ))
        .id();
    commands.insert_resource(OurColorPickerBrush(id));
}

fn destroy_brush(brush: Res<OurColorPickerBrush>, mut commands: Commands) {
    commands.entity(brush.0).despawn_descendants_recursive();
    commands.remove_resource::<OurColorPickerBrush>();
}

fn update_brush_viz(
    voxel_under_cursor: Res<CursorRayHit>,
    cursor_visible: Res<CursorVisible>,
    voxels_per_meter: Res<voxel::VoxelsPerMeter>,
    tool_color: Res<ToolColor>,
    our_color_picker: Res<OurColorPickerBrush>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut tool_brushes: Query<(&mut Transform, &mut Visibility, &MeshMaterial3d<StandardMaterial>)>,
) {
    let (mut transform, mut visibility, color) = tool_brushes.get_mut(our_color_picker.0).unwrap();
    if let Some(voxel) = voxel_under_cursor.ray_hit.filter(|_| cursor_visible.0) {
        transform.translation = voxel.entry_coords.to_world(*voxels_per_meter);
        transform.scale = Vec3::splat(voxel::VoxelSizeMeters::from(*voxels_per_meter).0);
        *visibility = Visibility::Visible;
        materials.get_mut(&color.0).unwrap().base_color = tool_color.base;
    } else {
        *visibility = Visibility::Hidden;
    }
}

fn on_use(
    mut tool_color: ResMut<ToolColor>,
    mut last_used_colors: ResMut<LastUsedColors>,
    cursor_ray_hit: Res<CursorRayHit>,
    chunks: Res<voxel::Chunks>,
    chunk_datas: Query<&voxel::ChunkData>,
) {
    let Some(hit_coords) = cursor_ray_hit.coords() else {
        return;
    };

    if let Some(voxel) = voxel::get(&chunks, &chunk_datas, hit_coords) {
        tool_color.base = voxel.color();
        last_used_colors.push(tool_color.base);
    }
}

pub fn ui_top_left_panel(ui: &mut bevy_egui::egui::Ui, world: &mut World) {
    let Some(hit_coords) = world.resource::<CursorRayHit>().coords() else {
        return;
    };

    match voxel::get_from_world(world, hit_coords).map(|v| v.color()) {
        Some(c) => {
            let (rect, _) = ui.allocate_exact_size(
                egui::vec2(ui.available_height(), ui.available_height()),
                egui::Sense::hover(),
            );
            ui.painter()
                .rect_filled(rect, egui::Rounding::ZERO, color::bevy_color_to_egui_hsv(c));
        }
        None => {
            ui.label("No voxel under cursor");
        }
    }
}
