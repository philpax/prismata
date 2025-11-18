use web_time::Duration;

use bevy::{camera::visibility::RenderLayers, prelude::*};
use bevy_egui::egui;

use crate::{
    raycast::{update_world_rayhits, CursorRayHit, RaycastIgnore},
    rendering::{AlphaPulse, MAIN_CAMERA_ONLY_LAYER},
    ui::CursorVisible,
    voxel,
};

use super::{
    brush_should_be_created, brush_should_be_destroyed, brush_size_range, is_in_use,
    scroll_size_system, LastUsedColors, Tool, ToolColor,
};

#[derive(Resource)]
pub struct OurTintBrush(pub Entity);

#[derive(Resource)]
pub struct OurTintBrushSettings {
    pub radius: f32,
    pub strength: f32,
}
impl Default for OurTintBrushSettings {
    fn default() -> Self {
        Self {
            radius: 0.0,
            strength: 0.25,
        }
    }
}

pub fn plugin(app: &mut App) {
    app.init_resource::<OurTintBrushSettings>().add_systems(
        Update,
        (
            scroll_size_system(Tool::Tint, |s: &mut OurTintBrushSettings| &mut s.radius),
            create_brush.run_if(brush_should_be_created::<OurTintBrush>(Tool::Tint)),
            destroy_brush.run_if(brush_should_be_destroyed::<OurTintBrush>(Tool::Tint)),
            update_brush_viz.run_if(resource_exists::<OurTintBrush>),
            on_use.run_if(is_in_use(Tool::Tint, Some(Duration::from_millis(30)))),
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
            Name::new("Tint Brush"),
            Mesh3d(meshes.add(Sphere { radius: 1.0 }.mesh().ico(5).unwrap())),
            MeshMaterial3d(materials.add(StandardMaterial {
                alpha_mode: AlphaMode::Blend,
                base_color: Color::linear_rgba(1.0, 1.0, 1.0, 1.0),
                unlit: true,
                ..default()
            })),
            Transform::default(),
            Visibility::Hidden,
            AlphaPulse::new(0.25, 1.5),
            RenderLayers::layer(MAIN_CAMERA_ONLY_LAYER as usize),
            RaycastIgnore,
        ))
        .id();
    commands.insert_resource(OurTintBrush(id));
}

fn destroy_brush(brush: Res<OurTintBrush>, mut commands: Commands) {
    commands.entity(brush.0).despawn();
    commands.remove_resource::<OurTintBrush>();
}

fn update_brush_viz(
    voxel_under_cursor: Res<CursorRayHit>,
    cursor_visible: Res<CursorVisible>,
    tool_color: Res<ToolColor>,
    voxels_per_meter: Res<voxel::VoxelsPerMeter>,
    our_tint_brush: Res<OurTintBrush>,
    settings: Res<OurTintBrushSettings>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut tool_brushes: Query<(&mut Transform, &mut Visibility, &MeshMaterial3d<StandardMaterial>)>,
) {
    let (mut transform, mut visibility, color) = tool_brushes.get_mut(our_tint_brush.0).unwrap();
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
    cursor_ray_hit: Res<CursorRayHit>,
    settings: Res<OurTintBrushSettings>,
    tool_color: Res<ToolColor>,
    voxels_per_meter: Res<voxel::VoxelsPerMeter>,
    mut last_used_colors: ResMut<LastUsedColors>,
    mut pending_dynamic_updates: MessageWriter<voxel::ChunkPendingDynamicUpdate>,
) {
    let Some(center) = cursor_ray_hit.coords() else {
        return;
    };

    last_used_colors.push(tool_color.base);

    let radius = settings.radius;
    let strength = settings.strength;
    let color = *tool_color;
    let radius_voxel = (radius * voxels_per_meter.0 as f32) as u32;
    let radius_voxel_sqr = radius_voxel * radius_voxel;

    voxel::update(
        &mut pending_dynamic_updates,
        *voxels_per_meter,
        voxel::UpdateParams {
            center,
            distance_meters: radius,
            should_allocate_chunk: |_, _| false,
            update: move |coords, old_voxel| {
                let distance_voxel_sqr = center.distance_squared(coords);
                let t = (1.0 - (distance_voxel_sqr as f32 / radius_voxel_sqr as f32)) * strength;
                if distance_voxel_sqr <= radius_voxel_sqr {
                    old_voxel.with_mixed_color(color.sample(), t)
                } else {
                    old_voxel
                }
            },
        },
    );
}

pub fn ui_top_left_panel(ui: &mut egui::Ui, world: &mut World) {
    let range = brush_size_range(world);
    let mut settings = world.resource_mut::<OurTintBrushSettings>();
    ui.add(
        egui::Slider::new(&mut settings.radius, range)
            .fixed_decimals(2)
            .text("Radius"),
    );
    ui.separator();
    ui.add(
        egui::Slider::new(&mut settings.strength, 0.01..=1.0)
            .fixed_decimals(2)
            .text("Strength"),
    );
}
